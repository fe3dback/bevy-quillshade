use super::*;
use shader_graph_core::ir::WPO_PIN;
use shader_project::fixtures;

use crate::tab::ViewKind;

fn project() -> (tempfile::TempDir, Editor) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"g\"\n").unwrap();
    std::fs::create_dir_all(dir.path().join("assets")).unwrap();
    let p = Project::open(dir.path()).unwrap();
    fixtures::save_all(&p).unwrap();
    let mut e = Editor::default();
    e.open_project(dir.path()).unwrap();
    (dir, e)
}

#[test]
fn constant_label_replaces_title() {
    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet")));
    let before: Vec<_> = e.doc().unwrap().asset.nodes.keys().cloned().collect();
    e.apply(Action::AddNode { kind: "stdlib::Constant".into(), pos: (0.0, 0.0), link: None, params: Vec::new() });
    let id = e.doc().unwrap().asset.nodes.keys().find(|k| !before.contains(k)).unwrap().clone();
    let title = |e: &Editor| crate::canvas::node_title(e, &e.doc().unwrap().asset, &id);
    let value_title = title(&e);
    let set = |s: &str| Action::SetValue {
        target: PropTarget::NodeLabel(id.clone()),
        value: Value::Str(s.into()),
        merge: true,
    };
    e.apply(set("rim"));
    assert_eq!(title(&e), "rim");
    e.apply(set(""));
    assert_eq!(title(&e), value_title);
}

#[test]
fn open_edit_save_regenerates() {
    let (dir, mut e) = project();
    let id = AssetId::new(AssetType::Material, "env/rock_wet");
    e.apply(Action::OpenAsset(id.clone()));
    assert_eq!(e.tabs.len(), 1);
    e.validate();
    assert!(!e.all_diagnostics().iter().any(Diagnostic::is_error), "{:?}", e.all_diagnostics());
    e.apply(Action::AddNode { kind: "stdlib::Time".into(), pos: (0.0, 0.0), link: None, params: Vec::new() });
    assert!(e.is_dirty());
    e.validate();
    assert!(e.all_diagnostics().iter().any(|d| d.code == Code::W101));
    e.apply(Action::Undo);
    assert!(!e.is_dirty());
    e.apply(Action::Redo);
    e.apply(Action::Save);
    assert!(!e.is_dirty());
    assert!(dir.path().join("assets/shaders/autogen/dir_env/mat_rock_wet.wesl").is_file());
    assert!(dir.path().join("crates/shaders/src/materials/dir_env/mat_rock_wet.rs").is_file());
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

    e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet")));
    e.apply(Action::AddNode { kind: "stdlib::Time".into(), pos: (0.0, 0.0), link: None, params: Vec::new() });
    e.apply(Action::Save);
    assert_eq!(rebuild(&e), before, "the Rust is unchanged");

    e.apply(Action::SetValue { target: PropTarget::Blend, value: Value::Str("Blend".into()), merge: false });
    e.apply(Action::Save);
    let last = e.messages.iter().rev().find(|m| m.text.contains("rebuild the game")).unwrap();
    assert_eq!(rebuild(&e), before + 1);
    assert_eq!(last.level, Level::Warning);
    assert!(last.text.contains("env/rock_wet"), "{}", last.text);
}

#[test]
fn add_node_from_wire_connects() {
    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet")));
    let out = e.doc().unwrap().asset.nodes.iter().find(|(_, n)| n.kind == "output::Pbr3d").unwrap().0.clone();
    e.apply(Action::Disconnect { to: (out.clone(), "roughness".into()) });
    let link = Link { node: out.clone(), pin: "roughness".into(), from_output: false };
    e.apply(Action::AddNode { kind: "stdlib::Constant".into(), pos: (0.0, 0.0), link: Some(link), params: Vec::new() });
    let d = e.doc().unwrap();
    let edge = d.asset.edge_into(&out, "roughness").expect("auto-connected");
    assert_eq!(d.asset.nodes[&edge.from.0].kind, "stdlib::Constant");
    // One undo removes node and wire.
    let constant = edge.from.0.clone();
    e.apply(Action::Undo);
    assert!(e.doc().unwrap().asset.edge_into(&out, "roughness").is_none());
    assert!(!e.doc().unwrap().asset.nodes.contains_key(&constant));
}

fn add_lerp(e: &mut Editor) -> NodeId {
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet")));
    e.apply(Action::AddNode { kind: "stdlib::Lerp".into(), pos: (-700.0, 0.0), link: None, params: Vec::new() });
    let d = e.doc().unwrap();
    d.asset.nodes.iter().find(|(_, n)| n.kind == "stdlib::Lerp").unwrap().0.clone()
}

#[test]
fn inline_drag_is_one_undo_step() {
    let (_dir, mut e) = project();
    let lerp = add_lerp(&mut e);
    let alpha = PropTarget::Input { node: lerp.clone(), pin: "alpha".into() };
    let set = |value: f32| Action::SetComponent { target: alpha.clone(), comp: 0, value, is_final: false };
    let input = |e: &Editor| e.doc().unwrap().asset.nodes[&lerp].inputs.get("alpha").cloned();
    for v in [0.2, 0.4, 0.6] {
        e.apply(set(v));
    }
    let props = e.ui.props;
    e.apply(Action::CommitInline);
    assert!(e.ui.props > props, "Properties redraws with the committed value");
    // The next drag is a separate undo step.
    e.apply(set(0.9));
    e.apply(Action::Undo);
    assert_eq!(input(&e), Some(Value::Float(0.6)));
    e.apply(Action::Undo);
    assert_eq!(input(&e), None);
}

/// Review Focus 4: a drag's merged edits (`SetValue { merge: true }`, what a non-final number input change
/// becomes) ending with a final `SetComponent` are one undo step, on a node input and on a global constant alike.
#[test]
fn number_drag_is_one_undo_step() {
    let (_dir, mut e) = project();
    let lerp = add_lerp(&mut e);
    let alpha = PropTarget::Input { node: lerp.clone(), pin: "alpha".into() };
    let input = |e: &Editor| e.doc().unwrap().asset.nodes[&lerp].inputs.get("alpha").cloned();
    for v in [0.2, 0.4, 0.6] {
        e.apply(Action::SetValue { target: alpha.clone(), value: Value::Float(v), merge: true });
    }
    e.apply(Action::SetComponent { target: alpha.clone(), comp: 0, value: 0.7, is_final: true });
    assert_eq!(input(&e), Some(Value::Float(0.7)));
    e.apply(Action::Undo);
    assert_eq!(input(&e), None, "one undo restores the value before the drag");

    e.apply(Action::OpenAsset(tuning()));
    let amount = PropTarget::Const("glow_amount".into());
    let before = e.current_value(&amount);
    for v in [2.0, 3.0] {
        e.apply(Action::SetValue { target: amount.clone(), value: Value::Float(v), merge: true });
    }
    e.apply(Action::SetComponent { target: amount.clone(), comp: 0, value: 4.0, is_final: true });
    // The next edit is a step of its own.
    e.apply(Action::SetComponent { target: amount.clone(), comp: 0, value: 5.0, is_final: true });
    e.apply(Action::Undo);
    assert_eq!(e.current_value(&amount), Some(Value::Float(4.0)));
    e.apply(Action::Undo);
    assert_eq!(e.current_value(&amount), before);
}

fn lerp_types(e: &Editor, lerp: &str) -> Vec<PinType> {
    let d = e.doc().unwrap();
    let sig = node_signature(&d.asset, lerp, &e.registry, &e.functions, &d.id.path).unwrap();
    ["a", "b", "alpha"].iter().map(|p| sig.input(p).unwrap().ty).chain([sig.outputs[0].ty]).collect()
}

#[test]
fn set_pin_type_resolves_poly() {
    let (_dir, mut e) = project();
    let lerp = add_lerp(&mut e);
    e.apply(Action::SetPinType { node: lerp.clone(), ty: PinType::Float3 });
    use PinType::*;
    assert_eq!(lerp_types(&e, &lerp), vec![Float3, Float3, Float, Float3]);
    e.apply(Action::SetPinType { node: lerp.clone(), ty: Float });
    assert_eq!(lerp_types(&e, &lerp), vec![Float, Float, Float, Float]);
    e.apply(Action::Undo);
    assert_eq!(lerp_types(&e, &lerp), vec![Float3, Float3, Float, Float3]);
}

#[test]
fn reset_input_removes_override() {
    let (_dir, mut e) = project();
    let lerp = add_lerp(&mut e);
    let alpha = PropTarget::Input { node: lerp.clone(), pin: "alpha".into() };
    e.apply(Action::SetComponent { target: alpha, comp: 0, value: 0.8, is_final: true });
    e.apply(Action::ResetInput { node: lerp.clone(), pin: "alpha".into() });
    let input = |e: &Editor| e.doc().unwrap().asset.nodes[&lerp].inputs.get("alpha").cloned();
    assert_eq!(input(&e), None);
    e.apply(Action::Undo);
    assert_eq!(input(&e), Some(Value::Float(0.8)));
}

#[test]
fn convert_keeps_wires_and_inputs_in_one_undo_step() {
    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet")));
    e.apply(Action::AddNode { kind: "stdlib::Add".into(), pos: (-700.0, 0.0), link: None, params: Vec::new() });
    let add = e.doc().unwrap().asset.nodes.iter().find(|(_, n)| n.kind == "stdlib::Add").unwrap().0.clone();
    e.apply(Action::SetValue {
        target: PropTarget::Input { node: add.clone(), pin: "b".into() },
        value: Value::Float(3.0),
        merge: false,
    });
    let lerp = add_lerp(&mut e);
    e.apply(Action::Connect { from: (add.clone(), "out".into()), to: (lerp.clone(), "a".into()) });
    e.apply(Action::Select([add.clone(), lerp.clone()].into()));
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
    e.apply(Action::Undo);
    assert_eq!(e.doc().unwrap().asset.nodes[&add].kind, "stdlib::Add");
}

fn output_node(e: &Editor) -> NodeId {
    e.doc().unwrap().asset.nodes.iter().find(|(_, n)| n.kind == "output::Pbr3d").unwrap().0.clone()
}

fn output_mode(e: &Editor) -> PbrOutputMode {
    match e.doc().unwrap().asset.kind {
        AssetKind::Material(MaterialKind::Pbr3d(s)) => s.output,
        _ => panic!("not a Pbr3d material"),
    }
}

fn add(e: &mut Editor, kind: &str) -> NodeId {
    let before: BTreeSet<NodeId> = e.doc().unwrap().asset.nodes.keys().cloned().collect();
    e.apply(Action::AddNode { kind: kind.into(), pos: (-900.0, 0.0), link: None, params: Vec::new() });
    e.doc().unwrap().asset.nodes.keys().find(|k| !before.contains(*k)).unwrap().clone()
}

fn set_output_mode(e: &mut Editor, mode: &str) {
    e.apply(Action::SetValue { target: PropTarget::OutputMode, value: Value::Str(mode.into()), merge: false });
}

fn pins_into(e: &Editor, node: &str) -> BTreeSet<String> {
    e.doc().unwrap().asset.edges.iter().filter(|x| x.to.0 == node).map(|x| x.to.1.clone()).collect()
}

/// Decision 0023: switching the Pbr3d output mode drops wires into pins the new layout lacks, in one undo step.
#[test]
fn output_mode_switch_prunes_edges_and_undoes() {
    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet")));
    let out = output_node(&e);
    let (v3, k) = (add(&mut e, "stdlib::Constant3Vector"), add(&mut e, "stdlib::Constant"));
    e.apply(Action::Connect { from: (v3, "out".into()), to: (out.clone(), "base_color".into()) });
    e.apply(Action::Connect { from: (k, "out".into()), to: (out.clone(), "roughness".into()) });
    e.apply(Action::SetValue {
        target: PropTarget::Input { node: out.clone(), pin: "specular".into() },
        value: Value::Float(0.3),
        merge: false,
    });
    let make = add(&mut e, "stdlib::MakeMaterialAttributesPbr");
    let before = pins_into(&e, &out);
    assert!(before.contains("base_color") && before.contains("roughness"), "{before:?}");

    set_output_mode(&mut e, "Wrapped");
    assert_eq!(output_mode(&e), PbrOutputMode::Wrapped);
    assert!(pins_into(&e, &out).iter().all(|p| p == WPO_PIN), "{:?}", pins_into(&e, &out));
    assert!(e.doc().unwrap().asset.nodes[&out].inputs.is_empty());
    e.apply(Action::Connect { from: (make, "attributes".into()), to: (out.clone(), "attributes".into()) });
    assert!(pins_into(&e, &out).contains("attributes"));

    set_output_mode(&mut e, "Default");
    assert_eq!(output_mode(&e), PbrOutputMode::Default);
    assert!(!pins_into(&e, &out).contains("attributes"));

    e.apply(Action::Undo);
    assert_eq!(output_mode(&e), PbrOutputMode::Wrapped);
    assert!(pins_into(&e, &out).contains("attributes"));
    e.apply(Action::Undo); // the attributes wire
    e.apply(Action::Undo); // the switch to `Wrapped`
    assert_eq!(output_mode(&e), PbrOutputMode::Default);
    assert_eq!(pins_into(&e, &out), before);
    assert_eq!(e.doc().unwrap().asset.nodes[&out].inputs.get("specular"), Some(&Value::Float(0.3)));
}

#[test]
fn wpo_edge_survives_mode_switch() {
    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet")));
    let out = output_node(&e);
    let v3 = add(&mut e, "stdlib::Constant3Vector");
    e.apply(Action::Connect { from: (v3.clone(), "out".into()), to: (out.clone(), WPO_PIN.into()) });
    set_output_mode(&mut e, "Wrapped");
    assert!(pins_into(&e, &out).contains(WPO_PIN));
    set_output_mode(&mut e, "Default");
    assert!(pins_into(&e, &out).contains(WPO_PIN));
    assert_eq!(e.current_value(&PropTarget::OutputMode), Some(Value::Str("Default".into())));
}

fn node_of(e: &Editor, kind: &str) -> NodeId {
    e.doc().unwrap().asset.nodes.iter().find(|(_, n)| n.kind == kind).unwrap().0.clone()
}

fn active_asset(e: &Editor) -> AssetId {
    e.tabs[e.active.unwrap()].owner().unwrap().clone()
}

/// Decision 0023: the file a node refers to (double-click, props "Open" button).
#[test]
fn referenced_asset_resolves_targets() {
    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet")));
    let call = node_of(&e, "function::common/wet_mask");
    assert_eq!(e.referenced_asset(&call), Some(AssetId::new(AssetType::Function, "common/wet_mask")));
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "fx/curve_bands")));
    let curve = node_of(&e, "stdlib::SampleCurve");
    let path = e.doc().unwrap().asset.nodes[&curve].params["curve"].as_str().to_string();
    assert_eq!(e.referenced_asset(&curve), Some(AssetId::new(AssetType::Curve, path)));
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "fx/global_tint")));
    let global = node_of(&e, "stdlib::GlobalConstF32");
    assert_eq!(e.referenced_asset(&global), Some(AssetId::new(AssetType::Consts, "common/tuning")));
}

#[test]
fn referenced_asset_none() {
    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet")));
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
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet")));
    let call = node_of(&e, "function::common/wet_mask");
    let target = e.referenced_asset(&call).unwrap();
    let tabs = e.tabs.len();
    e.apply(Action::OpenAsset(target.clone()));
    assert_eq!(active_asset(&e), target);
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet")));
    e.apply(Action::OpenAsset(target.clone()));
    assert_eq!(e.tabs.len(), tabs + 1);
    assert_eq!(active_asset(&e), target);
}

/// Design §5.5: what a node header's double click renames.
#[test]
fn rename_target_kinds() {
    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet")));
    let param = node_of(&e, "param::Scalar");
    assert_eq!(e.rename_target(&param), Some(PropTarget::Param { node: param.clone(), key: "name".into() }));
    let constant = add(&mut e, "stdlib::Constant");
    assert_eq!(e.rename_target(&constant), Some(PropTarget::NodeLabel(constant.clone())), "unlabelled Constant");
    let lerp = node_of(&e, "stdlib::Lerp");
    assert_eq!(e.rename_target(&lerp), None);
    let call = node_of(&e, "function::common/wet_mask");
    assert_eq!(e.rename_target(&call), None, "a function call opens its function instead");
    let wet_mask = Action::OpenAsset(AssetId::new(AssetType::Function, "common/wet_mask"));
    assert_eq!(e.open_action(&call), Some(wet_mask.clone()), "its body and its menu's Open in tab do");
    assert_eq!(e.header_action(&call), Some(wet_mask), "a reference first");
    let comment = |n: &str| Some(Action::TextDialog(PropTarget::NodeComment(n.to_string())));
    assert_eq!(e.header_action(&lerp), comment(&lerp), "no reference, no name: the comment");
    assert_eq!(e.open_action(&lerp), None);
    // A Named Route declaration has a name; its usages refer to it by node id, so a rename keeps them.
    let route = add(&mut e, "stdlib::NamedRoute");
    let route_name = PropTarget::Param { node: route.clone(), key: "name".into() };
    assert_eq!(e.rename_target(&route), Some(route_name.clone()));
    assert_eq!(e.header_action(&route), Some(Action::TextDialog(route_name)));
    let usage = add(&mut e, "stdlib::NamedRouteUsage");
    assert_eq!(e.rename_target(&usage), None, "a usage shows its declaration's name");
    assert_eq!(
        e.header_action(&param),
        Some(Action::TextDialog(PropTarget::Param { node: param, key: "name".into() }))
    );
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Function, "common/wet_mask")));
    let input = node_of(&e, "output::FunctionInput");
    assert_eq!(e.rename_target(&input), Some(PropTarget::Param { node: input, key: "name".into() }));
    let output = node_of(&e, "output::FunctionOutput");
    assert_eq!(e.rename_target(&output), Some(PropTarget::Param { node: output, key: "name".into() }));
}

/// Opens the rename dialog of `target` and submits `text`, as the dialog's OK / Enter does.
fn rename_through_dialog(e: &mut Editor, target: PropTarget, text: &str) {
    e.apply(Action::TextDialog(target.clone()));
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
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet")));
    let param = node_of(&e, "param::Scalar");
    let target = PropTarget::Param { node: param.clone(), key: "name".into() };
    let before = e.current_value(&target).unwrap();
    e.apply(Action::TextDialog(target.clone()));
    let Some(Dialog::Input { title, label, initial, .. }) = &e.dialog else { panic!("{:?}", e.dialog) };
    assert_eq!((title.as_str(), label.as_str()), ("Rename node", "Name"));
    assert_eq!(*initial, before.as_str(), "prefilled with the current name");
    let props = e.ui.props;
    rename_through_dialog(&mut e, target.clone(), "gloss");
    assert_eq!(e.current_value(&target), Some(Value::Str("gloss".into())));
    assert!(e.ui.props > props, "the right sidebar shows the new name");
    e.apply(Action::Undo);
    assert_eq!(e.current_value(&target), Some(before), "one undo step");
    assert!(!e.is_dirty());

    // A comment title goes the same way.
    let title = PropTarget::FrameTitle("c01".into());
    e.apply(Action::TextDialog(title.clone()));
    let Some(Dialog::Input { title: t, label, .. }) = &e.dialog else { panic!("{:?}", e.dialog) };
    assert_eq!((t.as_str(), label.as_str()), ("Comment", "Title"));
    rename_through_dialog(&mut e, title.clone(), "Wet rocks");
    assert_eq!(e.current_value(&title), Some(Value::Str("Wet rocks".into())));
}

#[test]
fn rename_dialog_empty_label_clears() {
    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet")));
    let constant = add(&mut e, "stdlib::Constant");
    let target = PropTarget::NodeLabel(constant.clone());
    rename_through_dialog(&mut e, target.clone(), "rim");
    assert_eq!(e.doc().unwrap().asset.nodes[&constant].label, "rim");
    e.apply(Action::TextDialog(target.clone()));
    assert!(matches!(&e.dialog, Some(Dialog::Input { initial, .. }) if initial == "rim"));
    e.apply(Action::Submit("  ".into()));
    assert_eq!(e.doc().unwrap().asset.nodes[&constant].label, "", "no label");
    let title = crate::canvas::node_title(&e, &e.doc().unwrap().asset, &constant);
    assert_ne!(title, "rim", "the title shows the value again");
}

#[test]
fn convert_drops_wires_and_inputs_of_missing_pins() {
    let (_dir, mut e) = project();
    let lerp = add_lerp(&mut e);
    e.apply(Action::AddNode { kind: "stdlib::Add".into(), pos: (-900.0, 0.0), link: None, params: Vec::new() });
    e.apply(Action::AddNode { kind: "stdlib::Time".into(), pos: (-1100.0, 0.0), link: None, params: Vec::new() });
    let find = |e: &Editor, k: &str| e.doc().unwrap().asset.nodes.iter().find(|(_, n)| n.kind == k).unwrap().0.clone();
    let (add, time) = (find(&e, "stdlib::Add"), find(&e, "stdlib::Time"));
    e.apply(Action::SetValue {
        target: PropTarget::Input { node: add.clone(), pin: "b".into() },
        value: Value::Float(3.0),
        merge: false,
    });
    e.apply(Action::Connect { from: (time, "out".into()), to: (add.clone(), "a".into()) });
    e.apply(Action::Connect { from: (add.clone(), "out".into()), to: (lerp.clone(), "a".into()) });
    e.apply(Action::ConvertNodes { nodes: vec![add.clone()], conv: Conversion::kind("stdlib::Abs") });
    let d = e.doc().unwrap();
    assert_eq!(d.asset.nodes[&add].kind, "stdlib::Abs");
    assert!(d.asset.nodes[&add].inputs.is_empty(), "`b` does not exist on Abs");
    assert!(d.asset.edge_into(&add, "a").is_none(), "the wire into `a` is dropped");
    assert!(d.asset.edge_into(&lerp, "a").is_some(), "`out` exists on both");
}

#[test]
fn node_menu_lists_convert_with_hotkeys_and_generic_rows() {
    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet")));
    e.apply(Action::AddNode { kind: "stdlib::Multiply".into(), pos: (0.0, 0.0), link: None, params: Vec::new() });
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
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet")));
    let call = node_of(&e, "function::common/wet_mask");
    let menu = |e: &Editor| e.node_menu(&call);
    let wet_mask = Action::OpenAsset(AssetId::new(AssetType::Function, "common/wet_mask"));
    assert!(menu(&e).contains(&("Open in tab".into(), wet_mask)));
    let target = PropTarget::NodeComment(call.clone());
    let dialog = Action::TextDialog(target.clone());
    assert!(menu(&e).contains(&("Add comment".into(), dialog.clone())));
    e.apply(dialog.clone());
    let Some(Dialog::Input { submit: Submit::SetText(t), initial, .. }) = e.dialog.clone() else { panic!() };
    assert_eq!((t, initial.as_str()), (target.clone(), ""));
    e.apply(Action::Submit("x".repeat(Node::COMMENT_MAX + 10)));
    let stored = |e: &Editor| e.doc().unwrap().asset.nodes[&call].comment.clone();
    assert_eq!(stored(&e).chars().count(), Node::COMMENT_MAX, "cut to the limit");
    assert!(menu(&e).contains(&("Edit comment".into(), dialog)));
    e.apply(Action::SetValue { target, value: Value::Str("wet\nlayer".into()), merge: false });
    assert_eq!(stored(&e), "wet\nlayer");
    e.apply(Action::Undo);
    assert_eq!(stored(&e).len(), Node::COMMENT_MAX, "one undo step per edit");
}

fn press_v(e: &mut Editor) {
    for a in e.hotkey_actions(Hotkey::plain(HotkeyKey::Letter('v'))) {
        e.apply(a);
    }
}

fn rock_wet_wesl(e: &Editor) -> PathBuf {
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
    e.apply(Action::Select([lerp.clone()].into()));
    press_v(&mut e);
    assert_eq!(e.doc().unwrap().asset.preview.node.as_deref(), Some(lerp.as_str()));
    assert_eq!(std::fs::read(&rust).unwrap(), rust_before, "a plain toggle leaves the Rust module alone");
    assert!(e.is_dirty(), "toggling the preview is a document edit");
    assert_eq!(e.tab().unwrap().title(), "shaders/env/rock_wet.material (preview)");
    assert_eq!(e.tab().unwrap().short_title(), "rock_wet (preview)");
    let previewed = std::fs::read_to_string(&wesl).unwrap();
    assert!(previewed.contains(&format!("// preview of node {lerp} (unlit)")), "{previewed}");
    let source = dir.path().join("shaders/env/rock_wet.material.ron");
    assert!(!std::fs::read_to_string(&source).unwrap().contains("preview"), "nothing is saved");
    e.apply(Action::Undo);
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
    let flag = |e: &Editor| e.tabs[e.doc_index(&rock).unwrap()].graph().unwrap().regen_from_memory;
    // Reload of an inactive tab (the watcher asks about any tab) and of the active one.
    for other_active in [true, false] {
        let lerp = add_lerp(&mut e);
        e.apply(Action::TogglePreview(lerp));
        assert!(flag(&e));
        assert!(std::fs::read_to_string(&wesl).unwrap().contains("// preview of node"));
        if other_active {
            e.apply(Action::OpenAsset(pulse()));
        }
        e.apply(Action::Reload(rock.clone()));
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
    e.apply(Action::TogglePreview(lerp));
    e.apply(Action::OpenAsset(twin));
    assert_eq!(
        crate::tab::labels(&e.tabs),
        ["shaders/env/rock_wet.material (preview)", "shaders/props/rock_wet.material"]
    );
    e.apply(Action::OpenAsset(pulse()));
    assert_eq!(crate::tab::labels(&e.tabs)[2], "pulse");
}

fn preview_messages(e: &Editor) -> usize {
    e.messages.iter().filter(|m| m.text.starts_with("Preview:")).count()
}

#[test]
fn saving_another_tab_keeps_an_unsaved_preview() {
    let (dir, mut e) = project();
    e.apply(Action::RegenAll);
    let wesl = rock_wet_wesl(&e);
    let rock = AssetId::new(AssetType::Material, "env/rock_wet");
    let lerp = add_lerp(&mut e);
    e.apply(Action::TogglePreview(lerp.clone()));
    let header = format!("// preview of node {lerp} (unlit)");
    // A trivial edit of a function the material calls, saved: the full regeneration keeps the preview.
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Function, "common/wet_mask")));
    e.apply(Action::SetValue { target: PropTarget::Description, value: Value::Str("tweak".into()), merge: false });
    e.apply(Action::Save);
    assert!(std::fs::read_to_string(&wesl).unwrap().contains(&header));
    e.apply(Action::RegenAll);
    assert!(std::fs::read_to_string(&wesl).unwrap().contains(&header));
    let i = e.doc_index(&rock).unwrap();
    assert!(e.tabs[i].graph().unwrap().regen_from_memory);
    // Saving the previewed tab stores the preview in the file and clears the flag.
    e.apply(Action::ActivateTab(i));
    e.apply(Action::Save);
    assert!(std::fs::read_to_string(dir.path().join("shaders/env/rock_wet.material.ron")).unwrap().contains(&lerp));
    assert!(!e.tabs[i].graph().unwrap().regen_from_memory);
    assert!(std::fs::read_to_string(&wesl).unwrap().contains(&header));
}

#[test]
fn a_preview_that_stopped_working_can_be_turned_off() {
    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet")));
    e.apply(Action::AddNode { kind: "stdlib::Not".into(), pos: (0.0, 0.0), link: None, params: Vec::new() });
    let not = e.doc().unwrap().asset.nodes.iter().find(|(_, n)| n.kind == "stdlib::Not").unwrap().0.clone();
    // As if stored in the file (or the node's output stopped converting to a color): W107 "preview ignored".
    e.doc_mut().unwrap().edit(|a| a.preview.node = Some(not.clone()));
    let row = e.node_menu(&not).into_iter().next().unwrap();
    assert_eq!(row, ("Stop preview\tV".to_string(), Action::TogglePreview(not.clone())));
    e.apply(Action::Select([not.clone()].into()));
    assert_eq!(e.hotkey_actions(Hotkey::plain(HotkeyKey::Letter('v'))), [Action::TogglePreview(not.clone())]);
    press_v(&mut e);
    assert!(e.doc().unwrap().asset.preview.node.is_none());
    // Off, it cannot be turned on again.
    assert!(e.hotkey_actions(Hotkey::plain(HotkeyKey::Letter('v'))).is_empty());
    assert_eq!(e.node_menu(&not)[0].0, "Exclusive preview (not available)");
}

#[test]
fn mode_change_without_a_previewed_node_does_not_regenerate() {
    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet")));
    let before = preview_messages(&e);
    e.apply(Action::SetValue { target: PropTarget::PreviewMode, value: Value::Str("Full".into()), merge: false });
    assert_eq!(e.doc().unwrap().asset.preview.mode, PreviewMode::Full);
    assert_eq!(preview_messages(&e), before);
    assert!(!e.tabs[e.active.unwrap()].graph().unwrap().regen_from_memory);
}

#[test]
fn preview_with_errors_elsewhere_keeps_file_and_warns() {
    let (_dir, mut e) = project();
    e.apply(Action::RegenAll);
    let wesl = rock_wet_wesl(&e);
    let normal = std::fs::read_to_string(&wesl).unwrap();
    let lerp = add_lerp(&mut e);
    // FlipBook without a texture: E004 (its `tex` pin is required).
    e.apply(Action::AddNode { kind: "stdlib::FlipBook".into(), pos: (-900.0, 0.0), link: None, params: Vec::new() });
    let tex = e.doc().unwrap().asset.nodes.iter().find(|(_, n)| n.kind == "stdlib::FlipBook").unwrap().0;
    let tex = tex.clone();
    e.apply(Action::Connect { from: (tex, "rgb".into()), to: (lerp.clone(), "a".into()) });
    let warnings = e.messages.iter().filter(|m| m.level == Level::Warning).count();
    e.apply(Action::TogglePreview(lerp.clone()));
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
    e.apply(Action::TogglePreview(lerp.clone()));
    e.apply(Action::SetValue { target: PropTarget::PreviewMode, value: Value::Str("Lit".into()), merge: false });
    assert_eq!(e.doc().unwrap().asset.preview.mode, PreviewMode::Lit);
    assert!(std::fs::read_to_string(&wesl).unwrap().contains(&format!("// preview of node {lerp} (lit)")));
    e.apply(Action::Select([lerp.clone()].into()));
    e.apply(Action::DeleteSelection);
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
    e.apply(Action::TogglePreview(lerp.clone()));
    let mode = |e: &Editor| e.doc().unwrap().asset.preview.mode;
    assert_eq!(mode(&e), PreviewMode::Unlit);
    e.apply(preview_mode_action(PreviewMode::Full));
    assert_eq!(mode(&e), PreviewMode::Full);
    assert!(e.is_dirty());
    e.apply(Action::Undo);
    assert_eq!(mode(&e), PreviewMode::Unlit, "one undo step");
    assert_eq!(e.doc().unwrap().asset.preview.node.as_deref(), Some(lerp.as_str()), "the preview stays on");
    e.apply(Action::Redo);
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
    assert_eq!(row.1, Action::TogglePreview(lerp.clone()));
    e.apply(row.1);
    assert_eq!(labels(&e, &lerp)[0], "Stop preview\tV");
    // The output node and nodes without a color-like output cannot be previewed.
    let out = e.doc().unwrap().asset.nodes.iter().find(|(_, n)| n.kind == "output::Pbr3d").unwrap().0.clone();
    let row = e.node_menu(&out).into_iter().next().unwrap();
    assert_eq!(row.0, "Exclusive preview (not available)");
    assert!(matches!(row.1, Action::Notify(_)));
    e.apply(Action::Select([out].into()));
    assert!(e.hotkey_actions(Hotkey::plain(HotkeyKey::Letter('v'))).is_empty());
    // `V` needs exactly one selected node.
    let all: BTreeSet<NodeId> = e.doc().unwrap().asset.nodes.keys().cloned().collect();
    e.apply(Action::Select(all));
    assert!(e.hotkey_actions(Hotkey::plain(HotkeyKey::Letter('v'))).is_empty());
    // Not offered in function graphs.
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Function, "common/wet_mask")));
    let n = e.doc().unwrap().asset.nodes.keys().next().unwrap().clone();
    assert!(!labels(&e, &n).iter().any(|l| l.contains("preview")));
    e.apply(Action::Select([n].into()));
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
    assert_eq!(pick, Action::SetPinType { node: lerp.clone(), ty: PinType::Float3 });

    e.apply(Action::AddNode { kind: "stdlib::Time".into(), pos: (-900.0, 0.0), link: None, params: Vec::new() });
    let d = e.doc().unwrap();
    let time = d.asset.nodes.iter().find(|(_, n)| n.kind == "stdlib::Time").unwrap().0.clone();
    e.apply(Action::Connect { from: (time, "out".into()), to: (lerp.clone(), "b".into()) });
    // A wire fixes the poly type: no type choice on any poly pin.
    assert_eq!(labels(&e, "a"), ["Reset to default"]);
    assert_eq!(labels(&e, "b"), ["Disconnect"]);
}

#[test]
fn component_edit_follows_resolved_pin_type() {
    let (_dir, mut e) = project();
    let lerp = add_lerp(&mut e);
    let a = PropTarget::Input { node: lerp.clone(), pin: "a".into() };
    e.apply(Action::SetValue { target: a.clone(), value: Value::Float(0.3), merge: false });
    e.apply(Action::AddNode {
        kind: "stdlib::Constant3Vector".into(),
        pos: (-900.0, 0.0),
        link: None,
        params: Vec::new(),
    });
    let d = e.doc().unwrap();
    let c3 = d.asset.nodes.iter().find(|(_, n)| n.kind == "stdlib::Constant3Vector").unwrap().0.clone();
    e.apply(Action::Connect { from: (c3, "out".into()), to: (lerp.clone(), "b".into()) });
    // A (stored as a scalar) now resolves to Float3: editing G must keep R and B.
    e.apply(Action::SetComponent { target: a.clone(), comp: 1, value: 0.9, is_final: true });
    assert_eq!(e.doc().unwrap().asset.nodes[&lerp].inputs.get("a"), Some(&Value::Float3([0.3, 0.9, 0.3])));
}

#[test]
fn committing_the_shown_rounded_value_keeps_precision() {
    let (_dir, mut e) = project();
    let lerp = add_lerp(&mut e);
    let alpha = PropTarget::Input { node: lerp.clone(), pin: "alpha".into() };
    e.apply(Action::SetValue { target: alpha.clone(), value: Value::Float(0.73803), merge: false });
    // A field shows 0.738; focus loss sends that text back.
    e.apply(Action::SetComponent { target: alpha.clone(), comp: 0, value: 0.738, is_final: true });
    assert_eq!(e.doc().unwrap().asset.nodes[&lerp].inputs.get("alpha"), Some(&Value::Float(0.73803)));
    e.apply(Action::SetComponent { target: alpha, comp: 0, value: 0.5, is_final: true });
    assert_eq!(e.doc().unwrap().asset.nodes[&lerp].inputs.get("alpha"), Some(&Value::Float(0.5)));
}

#[test]
fn create_rename_delete_assets() {
    let (dir, mut e) = project();
    e.apply(Action::NewAssetDialog(NewKind::Unlit3d));
    e.apply(Action::Submit("fx/glow".into()));
    let id = AssetId::new(AssetType::Material, "fx/glow");
    assert!(e.doc_index(&id).is_some());
    assert!(dir.path().join("assets/shaders/autogen/dir_fx/mat_glow.wesl").is_file());
    e.apply(Action::MoveTo(id.clone(), "fx2".into()));
    let moved = AssetId::new(AssetType::Material, "fx2/glow");
    assert!(e.doc_index(&moved).is_some());
    assert!(!dir.path().join("assets/shaders/autogen/dir_fx/mat_glow.wesl").exists());
    assert!(dir.path().join("assets/shaders/autogen/dir_fx2/mat_glow.wesl").is_file());
    e.apply(Action::Delete(moved.clone()));
    assert!(e.doc_index(&moved).is_none());
    assert!(!dir.path().join("assets/shaders/autogen/dir_fx2/mat_glow.wesl").exists());
    // Function rename updates open callers in memory.
    let rock = AssetId::new(AssetType::Material, "env/rock_wet");
    e.apply(Action::OpenAsset(rock.clone()));
    e.apply(Action::RenameDialog(AssetId::new(AssetType::Function, "common/wet_mask")));
    e.apply(Action::Submit("common/wetness".into()));
    let d = e.tabs[e.doc_index(&rock).unwrap()].graph().unwrap();
    assert!(d.asset.nodes.values().any(|n| n.kind == "function::common/wetness"));
    assert!(!d.is_dirty());
}

#[test]
fn close_dirty_tab_asks() {
    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet")));
    e.apply(Action::AddNode { kind: "stdlib::Time".into(), pos: (0.0, 0.0), link: None, params: Vec::new() });
    e.apply(Action::CloseTab(0));
    assert!(matches!(e.dialog, Some(Dialog::Confirm { .. })));
    assert_eq!(e.tabs.len(), 1);
    e.apply(Action::DiscardTab(0));
    assert!(e.tabs.is_empty());
    assert!(e.active.is_none());
}

/// Opens four materials; `dirty` ones get an unsaved node. Returns their names in tab order.
fn four_tabs(e: &mut Editor, dirty: &[usize]) -> Vec<String> {
    let names = ["env/rock_wet", "env/flag", "fx/hologram", "fx/coords"];
    for (i, n) in names.into_iter().enumerate() {
        e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, n)));
        if dirty.contains(&i) {
            e.apply(Action::AddNode { kind: "stdlib::Time".into(), pos: (0.0, 0.0), link: None, params: Vec::new() });
        }
    }
    names.iter().map(|n| format!("shaders/{n}.material")).collect()
}

fn titles(e: &Editor) -> Vec<String> {
    e.tabs.iter().map(Tab::title).collect()
}

#[test]
fn tab_menu_closes_groups_of_tabs() {
    let close = |which, at| Action::CloseTabs { which, at, unsaved: Unsaved::Ask };
    let (_dir, mut e) = project();
    let names = four_tabs(&mut e, &[]);
    e.apply(Action::ActivateTab(3));
    e.apply(close(CloseTabs::Left, 2));
    assert_eq!(titles(&e), names[2..]);
    assert_eq!(e.active, Some(1), "the active tab stays active");
    e.apply(close(CloseTabs::Right, 0));
    assert_eq!(titles(&e), names[2..3]);
    assert_eq!(e.active, Some(0), "the active tab closed: the last one left takes over");

    let (_dir, mut e) = project();
    four_tabs(&mut e, &[]);
    e.apply(Action::ActivateTab(0));
    e.apply(close(CloseTabs::Others, 2));
    assert_eq!(titles(&e), names[2..3]);
    assert_eq!(e.active, Some(0));
    e.apply(close(CloseTabs::All, 0));
    assert!(e.tabs.is_empty() && e.active.is_none());
}

#[test]
fn closing_unsaved_tabs_from_the_tab_menu_asks() {
    let (_dir, mut e) = project();
    let names = four_tabs(&mut e, &[1, 3]);
    e.apply(Action::CloseTabs { which: CloseTabs::Unmodified, at: 0, unsaved: Unsaved::Ask });
    assert!(e.dialog.is_none(), "only clean tabs: no question");
    assert_eq!(titles(&e), [names[1].clone(), names[3].clone()]);

    e.apply(Action::CloseTabs { which: CloseTabs::Others, at: 0, unsaved: Unsaved::Ask });
    let Some(Dialog::Confirm { buttons, .. }) = e.dialog.clone() else { panic!("asks") };
    assert_eq!(e.tabs.len(), 2);
    e.apply(buttons.last().unwrap().1.clone());
    assert!(e.dialog.is_none());
    assert_eq!(titles(&e), names[1..2]);
    assert!(e.tabs[0].doc().is_dirty(), "the kept tab keeps its changes");
    assert!(
        e.disk.assets[&AssetId::new(AssetType::Material, "fx/coords")]
            .as_ref()
            .unwrap()
            .nodes
            .values()
            .any(|n| n.kind == "stdlib::Time"),
        "Save saved the closed one"
    );

    e.apply(Action::CloseTabs { which: CloseTabs::All, at: 0, unsaved: Unsaved::Discard });
    assert!(e.tabs.is_empty());
}

#[test]
fn closing_the_project_remembers_its_tabs() {
    let (dir, mut e) = project();
    let names = four_tabs(&mut e, &[2]);
    e.apply(Action::CloseProject(Unsaved::Ask));
    assert!(matches!(e.dialog, Some(Dialog::Confirm { .. })), "unsaved changes: asks");
    assert!(e.project.is_some());
    e.apply(Action::CloseProject(Unsaved::Discard));
    assert!(e.project.is_none() && e.tabs.is_empty() && e.disk.ids().next().is_none());
    e.open_project(dir.path()).unwrap();
    assert_eq!(titles(&e), names);
    assert!(!e.is_dirty(), "discarded");
}

/// Opening another project with unsaved tabs asks Cancel / Discard / Save, as closing tabs does.
#[test]
fn switching_projects_asks_about_unsaved_tabs() {
    let (dir, mut e) = project();
    let (other, _) = project();
    let coords = AssetId::new(AssetType::Material, "fx/coords");
    let has_time = |dir: &Path| {
        std::fs::read_to_string(dir.join("shaders/fx/coords.material.ron")).unwrap().contains("stdlib::Time")
    };
    four_tabs(&mut e, &[3]);
    let ask = |e: &mut Editor| {
        e.apply(Action::OpenProject(other.path().into()));
        let Some(Dialog::Confirm { buttons, .. }) = e.dialog.clone() else { panic!("asks: {:?}", e.dialog) };
        let labels: Vec<&str> = buttons.iter().map(|(l, _)| l.as_str()).collect();
        assert_eq!(labels, ["Cancel", "Discard", "Save"]);
        buttons
    };
    let root = e.project.as_ref().unwrap().root.clone();
    let buttons = ask(&mut e);
    e.apply(buttons[0].1.clone());
    assert!(e.dialog.is_none() && e.is_dirty(), "Cancel keeps everything");
    assert_eq!(e.project.as_ref().unwrap().root, root);

    e.apply(Action::OpenProject(other.path().join("missing")));
    assert!(matches!(e.dialog, Some(Dialog::Message { .. })), "no project: the error, no question");
    e.apply(Action::CloseDialog);

    let buttons = ask(&mut e);
    e.apply(buttons[2].1.clone());
    assert!(has_time(dir.path()), "Save saved {coords:?}");
    assert_ne!(e.project.as_ref().unwrap().root, root, "and switched");

    four_tabs(&mut e, &[3]);
    e.apply(Action::OpenProject(dir.path().into()));
    let Some(Dialog::Confirm { buttons, .. }) = e.dialog.clone() else { panic!("asks") };
    e.apply(buttons[1].1.clone());
    assert!(!has_time(other.path()), "Discard saved nothing");
    assert_eq!(e.project.as_ref().unwrap().root, root, "and switched");
}

#[test]
fn ctrl_click_disconnects_every_wire_of_a_pin() {
    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet")));
    let d = e.doc().unwrap();
    let out = d.asset.nodes.iter().find(|(_, n)| n.kind == "output::Pbr3d").unwrap().0.clone();
    let into = d.asset.edges.iter().filter(|x| x.to.0 == out).count();
    assert!(into > 1);
    let edge = d.asset.edges.iter().find(|x| x.to.0 == out).unwrap().clone();
    let fan_out = |e: &Editor| e.doc().unwrap().asset.edges.iter().filter(|x| x.from == edge.from).count();
    assert!(fan_out(&e) > 0);
    e.apply(Action::DisconnectPin { node: edge.from.0.clone(), pin: edge.from.1.clone(), output: true });
    assert_eq!(fan_out(&e), 0);
    e.apply(Action::Undo);
    e.apply(Action::DisconnectPin { node: out.clone(), pin: edge.to.1.clone(), output: false });
    let d = e.doc().unwrap();
    assert!(d.asset.edge_into(&out, &edge.to.1).is_none());
    assert_eq!(d.asset.edges.iter().filter(|x| x.to.0 == out).count(), into - 1, "only that pin");
}

#[test]
fn picked_folder_submits_the_project_dialog() {
    let (dir, _) = project();
    let mut e = Editor::default();
    e.apply(Action::OpenProjectDialog);
    assert!(!e.pick_folder, "only our dialog opens: the native picker waits for Browse");
    e.apply(Action::CloseDialog);
    e.apply(Action::FolderPicked(dir.path().into()));
    assert!(e.project.is_none(), "dialog closed meanwhile: ignored");
    e.apply(Action::OpenProjectDialog);
    e.apply(Action::BrowseFolder);
    e.apply(Action::FolderPicked(dir.path().into()));
    assert!(e.project.is_some() && e.dialog.is_none() && !e.pick_folder);
}

#[test]
fn recent_project_closes_the_open_dialog() {
    let (dir, mut e) = project();
    e.apply(Action::OpenProjectDialog);
    e.apply(Action::OpenProject(dir.path().into()));
    assert!(e.project.is_some() && e.dialog.is_none(), "{:?}", e.dialog);
    e.apply(Action::OpenProjectDialog);
    e.apply(Action::OpenProject(dir.path().join("missing")));
    assert!(matches!(e.dialog, Some(Dialog::Message { .. })), "the error replaces it: {:?}", e.dialog);
}

#[test]
fn forget_recent_keeps_the_project_open() {
    let (_dir, mut e) = project();
    let root = e.project.as_ref().unwrap().root.clone();
    assert_eq!(e.config.recent_projects, std::slice::from_ref(&root));
    e.apply(Action::ForgetRecent(root));
    assert!(e.config.recent_projects.is_empty());
    assert!(e.project.is_some(), "only the list entry goes");
}

#[test]
fn new_project_sets_up_a_game_crate() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = Editor::default();
    e.apply(Action::NewProjectDialog);
    e.apply(Action::Submit(dir.path().display().to_string()));
    assert!(e.project.is_none(), "no Cargo.toml: refused");
    assert!(matches!(e.dialog, Some(Dialog::Message { .. })));
    std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"game\"\n").unwrap();
    e.apply(Action::NewProject(dir.path().into()));
    let root = e.project.as_ref().unwrap().root.clone();
    assert!(root.join("assets").is_dir() && root.join("shaders").is_dir());
    assert_eq!(e.config.recent_projects, [root]);
}

#[test]
fn tabs_are_restored_on_next_launch() {
    let (dir, mut e) = project();
    let config = dir.path().join("config/config.ron");
    e.config_path = Some(config.clone());
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet")));
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "fx/hologram")));
    e.regenerate();
    e.apply(Action::ShowGenerated(AssetId::new(AssetType::Material, "fx/hologram"), Lang::Rust));
    e.apply(Action::ActivateTab(0));
    e.apply(Action::Quit);
    assert!(e.quit);

    let mut next = Editor::with_config(UserConfig::load(&config), Some(config));
    next.open_project(dir.path()).unwrap();
    let open: Vec<_> = next.tabs.iter().map(Tab::title).collect();
    assert_eq!(
        open,
        [
            "shaders/env/rock_wet.material",
            "shaders/fx/hologram.material",
            "crates/shaders/src/materials/dir_fx/mat_hologram.rs"
        ]
    );
    assert_eq!(next.active, Some(0));
}

/// View → Show console / Show properties (design §3): each toggle flips its flag and saves the user config, which the
/// next launch reads (Review Focus 5).
#[test]
fn toggle_log_and_properties_persist() {
    let (dir, mut e) = project();
    let config = dir.path().join("config/config.ron");
    e.config_path = Some(config.clone());
    assert!(e.config.show_log && e.config.show_properties, "shown by default");
    e.apply(Action::ToggleLog);
    assert!(!e.config.show_log && e.config.show_properties);
    assert_eq!(UserConfig::load(&config), e.config, "saved");
    e.apply(Action::ToggleProperties);
    assert!(!e.config.show_properties);
    let saved = UserConfig::load(&config);
    assert!(!saved.show_log && !saved.show_properties);
    e.apply(Action::ToggleLog);
    assert!(UserConfig::load(&config).show_log, "toggled back");
    let next = Editor::with_config(UserConfig::load(&config), Some(config));
    assert!(next.config.show_log && !next.config.show_properties);
}

/// Zen mode (design §3, Review Focus 3): it hides every panel while on; Ctrl+3 in Zen flips its own flag only; Zen
/// off shows exactly the panels whose flags are on. Each step is saved at once.
#[test]
fn zen_hides_panels_and_restores_flags() {
    use crate::config::Panel::{Console, LeftSidebar, Log, Properties};
    let (dir, mut e) = project();
    let config = dir.path().join("config/config.ron");
    e.config_path = Some(config.clone());
    e.apply(Action::ToggleLog);
    let shown = |e: &Editor| [LeftSidebar, Console, Log, Properties].map(|p| e.config.panel_visible(p));
    assert_eq!(shown(&e), [true, true, false, true], "left on, log off, properties on");

    e.apply(Action::ToggleZen);
    assert!(e.config.zen_mode);
    assert_eq!(shown(&e), [false; 4], "Zen hides every panel, the console header too");
    assert_eq!(UserConfig::load(&config), e.config, "Zen saved");

    e.apply(Action::ToggleProperties);
    assert!(!e.config.show_properties, "Ctrl+3 in Zen flips its flag");
    assert_eq!(shown(&e), [false; 4], "still hidden");
    assert_eq!(UserConfig::load(&config), e.config, "the flag saved");

    e.apply(Action::ToggleZen);
    assert!(!e.config.zen_mode);
    assert_eq!(shown(&e), [true, true, false, false], "left shown, the log header only, properties hidden");
    assert_eq!(UserConfig::load(&config), e.config, "Zen off saved");

    e.apply(Action::ToggleLeftSidebar);
    assert!(!e.config.show_left_sidebar);
    assert_eq!(UserConfig::load(&config), e.config, "Ctrl+1 saved");
}

#[test]
fn generated_files_open_in_text_tabs() {
    let (_dir, mut e) = project();
    let rock = AssetId::new(AssetType::Material, "env/rock_wet");
    let mask = AssetId::new(AssetType::Function, "common/wet_mask");
    // Not generated yet: a warning instead of an empty tab.
    e.apply(Action::ShowGenerated(rock.clone(), Lang::Wesl));
    assert!(e.tabs.is_empty());
    e.regenerate();
    e.apply(Action::OpenAsset(rock.clone()));
    e.apply(Action::ShowGenerated(rock.clone(), Lang::Wesl));
    assert_eq!(e.tabs.len(), 2);
    assert_eq!(e.active, Some(1));
    assert!(e.doc().is_none(), "graph commands must not reach a text tab");
    let t = e.tab().and_then(Tab::text).unwrap();
    assert_eq!(t.rel, "assets/shaders/autogen/dir_env/mat_rock_wet.wesl");
    assert!(t.text.as_deref().unwrap().starts_with("// AUTOGENERATED"));
    // Showing it again activates the open tab.
    e.apply(Action::ActivateTab(0));
    e.apply(Action::ShowGenerated(rock.clone(), Lang::Wesl));
    assert_eq!((e.tabs.len(), e.active), (2, Some(1)));
    e.apply(Action::Copy);
    assert!(e.copy_text.as_deref().unwrap().contains("fn fragment"));
    // Functions have no Rust module.
    e.apply(Action::ShowGenerated(mask.clone(), Lang::Rust));
    assert_eq!(e.tabs.len(), 2);
    // Regeneration refreshes the text; deleting the asset closes all of its tabs.
    e.apply(Action::ShowGenerated(mask.clone(), Lang::Wesl));
    let (path, original) = (e.tabs[1].text().unwrap().path.clone(), e.tabs[1].text().unwrap().text.clone());
    std::fs::write(&path, format!("{}// stale\n", original.as_deref().unwrap())).unwrap();
    e.apply(Action::ActivateTab(1));
    assert!(e.tabs[1].text().unwrap().text.as_deref().unwrap().ends_with("// stale\n"), "activation re-reads");
    let (rock_rev, mask_rev) = (e.tabs[1].text().unwrap().revision, e.tabs[2].text().unwrap().revision);
    e.apply(Action::RegenAll);
    assert_eq!(e.tabs[1].text().unwrap().text, original);
    assert!(e.tabs[1].text().unwrap().revision > rock_rev);
    assert_eq!(e.tabs[2].text().unwrap().revision, mask_rev, "an unchanged file keeps its view");
    e.apply(Action::Delete(rock));
    assert_eq!(e.tabs.len(), 1);
    assert_eq!(e.tabs[0].owner(), Some(&mask));
}

/// Regeneration rewriting the file of the active text tab rebuilds its right sidebar (the "N lines" row is keyed on
/// `ui.props`).
#[test]
fn regenerated_active_text_tab_refreshes_sidebar() {
    let (_dir, mut e) = project();
    e.regenerate();
    let rock = AssetId::new(AssetType::Material, "env/rock_wet");
    e.apply(Action::OpenAsset(rock.clone()));
    e.apply(Action::ShowGenerated(rock, Lang::Wesl));
    let lines = |e: &Editor| e.tabs[1].text().unwrap().text.as_deref().unwrap().lines().count();
    let before = lines(&e);
    // The graph stripped down to its output node, saved without regenerating.
    e.apply(Action::ActivateTab(0));
    e.apply(Action::SelectAll);
    e.apply(Action::DeleteSelection);
    assert!(e.save(0));
    e.apply(Action::ActivateTab(1));
    let props = e.ui.props;
    e.apply(Action::RegenAll);
    assert_ne!(lines(&e), before, "the WESL line count changed");
    assert_ne!(e.ui.props, props, "the active text tab's sidebar is rebuilt");
    // A reload of an inactive text tab leaves the sidebar alone.
    e.apply(Action::ActivateTab(0));
    let (path, text) = (e.tabs[1].text().unwrap().path.clone(), e.tabs[1].text().unwrap().text.clone());
    std::fs::write(&path, format!("{}// stale\n", text.as_deref().unwrap())).unwrap();
    let props = e.ui.props;
    e.apply(Action::RegenAll);
    assert_eq!(e.tabs[1].text().unwrap().text, text, "reloaded");
    assert_eq!(e.ui.props, props);
}

#[test]
fn clicking_a_diagnostic_opens_its_asset() {
    let (_dir, mut e) = project();
    let id = AssetId::new(AssetType::Material, "env/rock_wet");
    let node = e.disk.assets[&id].as_ref().unwrap().nodes.keys().next().unwrap().clone();
    let diag = Diagnostic::new(Code::W101, "x").in_asset(id.source_rel()).at_node(node.clone());
    e.apply(Action::FocusDiagnostic(diag));
    assert_eq!(e.doc().map(|d| &d.id), Some(&id));
    assert!(e.doc().unwrap().selection.contains(&node));
}

const ROUTE_DECL: &str = "stdlib::NamedRoute";

/// The palette's rows for `link` and `query` as (kind, name), in the order it lists them.
fn palette_rows(e: &Editor, link: Option<&Link>, query: &str) -> Vec<(String, String)> {
    let entries = crate::ui::palette::entries_for(e, link);
    let rows = crate::ui::palette::results_for(e, link, &entries, query);
    rows.into_iter().map(|i| (entries[i].kind.clone(), entries[i].name.clone())).collect()
}

/// The palette's entries as they were before design §6.4: every node, the route usages, filtered by the wire.
fn palette_before(e: &Editor, link: Option<&Link>) -> Vec<search::Entry> {
    let d = e.doc().unwrap();
    let mut entries = search::entries(&e.registry, &e.functions, d.asset.kind, &d.id.path);
    entries.extend(search::route_entries(&e.registry, &d.asset));
    entries.sort_by(|a, b| (&a.category, &a.name).cmp(&(&b.category, &b.name)));
    if let Some(link) = link {
        let t = e.pin_type(&link.node, &link.pin, link.from_output).unwrap();
        let end = if link.from_output { WireEnd::FromOutput(t) } else { WireEnd::FromInput(t) };
        entries.retain(|x| search::compatible_pin(&e.registry, &e.functions, d.asset.kind, &x.kind, end).is_some());
    }
    entries
}

/// The rows as the palette listed them before design §6.4 (plain search order).
fn palette_rows_before(e: &Editor, link: Option<&Link>, query: &str) -> Vec<(String, String)> {
    let entries = palette_before(e, link);
    search::search(&entries, query).into_iter().map(|x| (x.kind.clone(), x.name.clone())).collect()
}

fn scalar_output(e: &Editor) -> Link {
    let param = node_of(e, "param::Scalar");
    assert_eq!(e.pin_type(&param, "out", true), Some(PinType::Float));
    Link { node: param, pin: "out".into(), from_output: true }
}

/// Design §6.4: after a wire from an output the existing Named Route declaration entry wins ties on the score (first
/// with an empty query or "route", below better matches such as DotProduct for "dot"); entries and filters are
/// unchanged.
#[test]
fn output_wire_drop_palette_starts_with_named_route_declaration() {
    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet")));
    let link = scalar_output(&e);
    assert_eq!(crate::ui::palette::entries_for(&e, Some(&link)), palette_before(&e, Some(&link)), "same entries");
    let rows = palette_rows(&e, Some(&link), "");
    assert_eq!(rows[0].0, ROUTE_DECL);
    let before = palette_rows_before(&e, Some(&link), "");
    assert_ne!(before[0].0, ROUTE_DECL, "it was not first before");
    let rest: Vec<_> = before.iter().filter(|r| r.0 != ROUTE_DECL).cloned().collect();
    assert_eq!(rows[1..], rest[..], "the others keep their order");
    assert_eq!(palette_rows(&e, Some(&link), "route")[0].0, ROUTE_DECL);
    // A tie-break after the score: better matches stay above it ("dot" is a keyword of Reroute and DotProduct, a
    // mere subsequence of "namedroute"), in the plain order.
    let dot = palette_rows(&e, Some(&link), "dot");
    assert_ne!(dot[0].0, ROUTE_DECL, "{dot:?}");
    let at = |kind: &str| dot.iter().position(|r| r.0 == kind).unwrap();
    assert!(at("stdlib::DotProduct") < at(ROUTE_DECL), "{dot:?}");
    assert_eq!(dot, palette_rows_before(&e, Some(&link), "dot"));
    let mul = palette_rows(&e, Some(&link), "mul");
    assert!(!mul.iter().any(|r| r.0 == ROUTE_DECL), "the query filters it out: {mul:?}");
    assert_eq!(mul, palette_rows_before(&e, Some(&link), "mul"));
}

#[test]
fn input_wire_drop_palette_unchanged() {
    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet")));
    let out = node_of(&e, "output::Pbr3d");
    let link = Link { node: out, pin: "roughness".into(), from_output: false };
    assert_eq!(crate::ui::palette::entries_for(&e, Some(&link)), palette_before(&e, Some(&link)));
    for q in ["", "route", "mul"] {
        assert_eq!(palette_rows(&e, Some(&link), q), palette_rows_before(&e, Some(&link), q), "query {q:?}");
    }
    assert!(palette_rows(&e, Some(&link), "").iter().skip(1).any(|r| r.0 == ROUTE_DECL), "listed, not first");
}

#[test]
fn palette_without_link_unchanged() {
    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet")));
    assert_eq!(crate::ui::palette::entries_for(&e, None), palette_before(&e, None));
    for q in ["", "route", "mul"] {
        assert_eq!(palette_rows(&e, None, q), palette_rows_before(&e, None, q), "query {q:?}");
    }
    assert_ne!(palette_rows(&e, None, "")[0].0, ROUTE_DECL);
}

/// Design §6.4: a Named Route declaration picked in the palette (from an output wire or not) is added, wired from
/// the output in one undo step, and its rename dialog opens; cancelling keeps the default name.
#[test]
fn palette_named_route_declaration_opens_rename() {
    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet")));
    let output = scalar_output(&e);
    let pick = |e: &mut Editor, link: Option<&Link>, kind: &str| -> NodeId {
        let entries = crate::ui::palette::entries_for(e, link);
        let entry = entries.iter().find(|x| x.kind == kind).unwrap().clone();
        let before: BTreeSet<NodeId> = e.doc().unwrap().asset.nodes.keys().cloned().collect();
        e.apply(crate::ui::palette::pick_action(e, &entry, (123.0, 47.0), link.cloned()));
        let d = e.doc().unwrap();
        let new: Vec<_> = d.asset.nodes.keys().filter(|k| !before.contains(*k)).cloned().collect();
        assert_eq!(new.len(), 1, "{new:?}");
        new[0].clone()
    };
    let rename_open = |e: &Editor, route: &str| {
        let target = PropTarget::Param { node: route.to_string(), key: "name".into() };
        matches!(&e.dialog, Some(Dialog::Input { submit: Submit::SetText(t), initial, .. })
            if *t == target && initial == "route")
    };

    // From an output wire.
    let route = pick(&mut e, Some(&output), ROUTE_DECL);
    let d = e.doc().unwrap();
    assert_eq!(d.asset.nodes[&route].pos, (120.0, 50.0), "at the drop point, snapped");
    let edge = d.asset.edge_into(&route, "in").expect("wired from the output");
    assert_eq!(edge.from, (output.node.clone(), "out".to_string()));
    assert!(rename_open(&e, &route), "{:?}", e.dialog);
    let target = PropTarget::Param { node: route.clone(), key: "name".into() };
    // Cancel: the node keeps its default name.
    e.apply(Action::CloseDialog);
    assert_eq!(e.current_value(&target), Some(Value::Str("route".into())));
    // One undo step removes node and wire.
    e.apply(Action::Undo);
    assert!(!e.doc().unwrap().asset.nodes.contains_key(&route));
    assert!(!e.is_dirty());

    // Without a link (Space / RMB): the dialog opens too; submitting renames it (its own undo step).
    let route = pick(&mut e, None, ROUTE_DECL);
    assert!(rename_open(&e, &route), "{:?}", e.dialog);
    let target = PropTarget::Param { node: route.clone(), key: "name".into() };
    e.apply(Action::Submit("wet".into()));
    assert_eq!(e.dialog, None);
    assert_eq!(e.current_value(&target), Some(Value::Str("wet".into())));
    e.apply(Action::Undo);
    assert_eq!(e.current_value(&target), Some(Value::Str("route".into())));
    e.apply(Action::Undo);
    assert!(!e.doc().unwrap().asset.nodes.contains_key(&route));

    // Other nodes open no dialog.
    pick(&mut e, Some(&output), "stdlib::Multiply");
    assert_eq!(e.dialog, None);
}

#[test]
fn named_routes_in_the_editor() {
    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet")));
    e.apply(Action::AddNode { kind: "stdlib::NamedRoute".into(), pos: (0.0, 0.0), link: None, params: Vec::new() });
    let decl = e.doc().unwrap().selection.first().unwrap().clone();

    // The palette offers one usage entry per declaration, preset to it.
    let entries = search::route_entries(&e.registry, &e.doc().unwrap().asset);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "Named Route Usage: route");
    let params = entries[0].params.clone();
    e.apply(Action::AddNode { kind: entries[0].kind.clone(), pos: (0.0, 0.0), link: None, params });
    let usage = e.doc().unwrap().selection.first().unwrap().clone();
    let d = e.doc().unwrap();
    assert_eq!(d.asset.nodes[&usage].params["route"], Value::Str(decl.clone()));
    assert_eq!(crate::canvas::node_title(&e, &d.asset, &usage), "route (Named Route Usage)");

    // usage → declaration would close a loop through the route.
    let err = connection_error(&d.asset, (&usage, "out"), (&decl, "in"), &e.registry, &e.functions, &d.id.path);
    assert!(err.unwrap().contains("E002"));

    // Copied with its declaration into the same graph, the declaration becomes a usage too and both keep the
    // original (decision 0038; a duplicated declaration in another graph is followed, see `doc` tests).
    e.apply(Action::Select([decl.clone(), usage.clone()].into()));
    e.apply(Action::Duplicate);
    let d = e.doc().unwrap();
    assert_eq!(d.selection.len(), 2);
    for id in &d.selection {
        assert_eq!(d.asset.nodes[id].params["route"], Value::Str(decl.clone()));
    }
    e.apply(Action::Select([usage.clone()].into()));
    e.apply(Action::Duplicate);
    let d = e.doc().unwrap();
    let copy = d.selection.first().unwrap();
    assert_eq!(d.asset.nodes[copy].params["route"], Value::Str(decl));
}

fn pulse() -> AssetId {
    AssetId::new(AssetType::Curve, "common/pulse")
}

fn points(e: &Editor) -> Vec<(f32, f32)> {
    e.curve_doc().unwrap().curve.points.clone()
}

#[test]
fn tab_labels_are_short_unless_shared() {
    let (_dir, mut e) = project();
    e.regenerate();
    let rock = AssetId::new(AssetType::Material, "env/rock_wet");
    e.apply(Action::OpenAsset(rock.clone()));
    e.apply(Action::OpenAsset(pulse()));
    assert_eq!(crate::tab::labels(&e.tabs), ["rock_wet", "pulse"]);
    // Generated files are named `mat_*` (decision 0020): no clash with the graph tab.
    e.apply(Action::ShowGenerated(rock.clone(), Lang::Wesl));
    assert_eq!(crate::tab::labels(&e.tabs), ["rock_wet", "pulse", "mat_rock_wet"]);
    e.apply(Action::ShowGenerated(rock, Lang::Rust));
    assert_eq!(
        crate::tab::labels(&e.tabs),
        [
            "rock_wet",
            "pulse",
            "assets/shaders/autogen/dir_env/mat_rock_wet.wesl",
            "crates/shaders/src/materials/dir_env/mat_rock_wet.rs"
        ]
    );
}

#[test]
fn curve_tab_edits_undo_clipboard_and_save() {
    let (dir, mut e) = project();
    e.apply(Action::OpenAsset(pulse()));
    assert!(e.doc().is_none(), "graph commands must not reach a curve tab");
    assert_eq!(e.tab().unwrap().title(), "shaders/common/pulse.curve");
    let n = points(&e).len();

    e.apply(Action::Curve(CurveAction::Add((0.6, 0.9))));
    assert_eq!(points(&e).len(), n + 1);
    assert!(e.is_dirty());
    e.apply(Action::Undo);
    assert_eq!(points(&e).len(), n);
    assert!(!e.is_dirty());
    e.apply(Action::Redo);

    // Copy the new point, paste it twice, then select all and delete everything.
    e.apply(Action::Copy);
    e.apply(Action::Paste(None));
    assert_eq!(points(&e).len(), n + 2);
    assert_eq!(e.curve_doc().unwrap().selection.len(), 1, "the pasted point is selected");
    e.apply(Action::SelectAll);
    assert_eq!(e.curve_doc().unwrap().selection.len(), n + 2);
    e.apply(Action::DeleteSelection);
    assert!(points(&e).is_empty());
    e.apply(Action::Undo);
    assert_eq!(points(&e).len(), n + 2);

    // Properties edit the selected point; the preview switch does not touch the file.
    e.apply(Action::Curve(CurveAction::Select([0].into())));
    let target = PropTarget::CurvePoint(0);
    e.apply(Action::SetComponent { target: target.clone(), comp: 1, value: 0.5, is_final: true });
    assert_eq!(points(&e)[0], (0.0, 0.5));
    let saved_rev = e.curve_doc().unwrap().saved.clone();
    e.apply(Action::SetValue { target: PropTarget::CurvePreview, value: Value::Str("Simple".into()), merge: false });
    assert_eq!(e.curve_doc().unwrap().preview, Interp::Simple);
    assert_eq!(e.curve_doc().unwrap().saved, saved_rev);

    e.apply(Action::Save);
    assert!(!e.is_dirty());
    let wesl =
        std::fs::read_to_string(dir.path().join("assets/shaders/autogen/curves/dir_common/crv_pulse.wesl")).unwrap();
    assert!(wesl.contains("vec3<f32>(0.0, 0.5, "), "{wesl}");
    let on_disk = assets::load_curve(e.project.as_ref().unwrap(), &pulse()).unwrap();
    assert_eq!(on_disk.points, points(&e));
}

#[test]
fn nudges_merge_until_the_selection_changes() {
    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(pulse()));
    let start = points(&e);
    e.apply(Action::Curve(CurveAction::Select([1].into())));
    e.apply(Action::Curve(CurveAction::Nudge((0.0, -0.1))));
    e.apply(Action::Curve(CurveAction::Nudge((0.0, -0.1))));
    e.apply(Action::Curve(CurveAction::Select([2].into())));
    e.apply(Action::Curve(CurveAction::Nudge((0.0, 0.1))));
    e.apply(Action::Undo);
    assert!((points(&e)[1].1 - (start[1].1 - 0.2)).abs() < 1e-6);
    assert_eq!(points(&e)[2], start[2]);
    e.apply(Action::Undo);
    assert_eq!(points(&e), start);
}

fn curve_range(e: &Editor) -> (f32, f32) {
    let c = &e.curve_doc().unwrap().curve;
    (c.min, c.max)
}

fn set_range(e: &mut Editor, comp: usize, value: f32, is_final: bool) {
    e.apply(Action::SetComponent { target: PropTarget::CurveRange, comp, value, is_final });
}

#[test]
fn curve_range_edits_are_undoable_and_checked() {
    let (dir, mut e) = project();
    e.apply(Action::OpenAsset(pulse()));
    let start = points(&e);
    assert_eq!(e.current_value(&PropTarget::CurveRange), Some(Value::Float2([0.0, 1.0])));

    // A Max scrub is one undo step; Min typed is another; the points stay normalized.
    for v in [2.0, 3.0, 4.0] {
        set_range(&mut e, 1, v, false);
    }
    set_range(&mut e, 1, 4.0, true);
    set_range(&mut e, 0, -2.0, true);
    assert_eq!(curve_range(&e), (-2.0, 4.0));
    assert_eq!(points(&e), start);
    assert!(e.is_dirty());

    // `min >= max` is refused with a warning and changes nothing.
    let warnings = e.messages.iter().filter(|m| m.level == Level::Warning).count();
    set_range(&mut e, 0, 4.0, true);
    set_range(&mut e, 1, -3.0, true);
    assert_eq!(curve_range(&e), (-2.0, 4.0));
    let new: Vec<&Message> = e.messages.iter().filter(|m| m.level == Level::Warning).skip(warnings).collect();
    assert_eq!(new.len(), 2, "{new:?}");
    assert!(new.iter().all(|m| m.text.contains("min")), "{new:?}");

    e.apply(Action::Undo);
    assert_eq!(curve_range(&e), (0.0, 4.0));
    e.apply(Action::Undo);
    assert_eq!(curve_range(&e), (0.0, 1.0));
    e.apply(Action::Redo);
    e.apply(Action::Redo);
    assert_eq!(curve_range(&e), (-2.0, 4.0));

    // The selected point's Value field shows and edits real units; T stays normalized.
    e.apply(Action::Curve(CurveAction::Select([1].into())));
    let target = PropTarget::CurvePoint(1);
    assert_eq!(e.current_value(&target), Some(Value::Float2([0.25, 4.0])));
    e.apply(Action::SetComponent { target: target.clone(), comp: 1, value: 1.0, is_final: true });
    assert_eq!(points(&e)[1], (0.25, 0.5));
    e.apply(Action::SetComponent { target: target.clone(), comp: 0, value: 0.2, is_final: true });
    assert_eq!(points(&e)[1], (0.2, 0.5), "a T edit keeps the stored value exactly");
    // A typed value outside the range is clamped to it, like a dragged point.
    e.apply(Action::SetComponent { target, comp: 1, value: 9.0, is_final: true });
    assert_eq!(points(&e)[1], (0.2, 1.0));

    e.apply(Action::Save);
    let on_disk = assets::load_curve(e.project.as_ref().unwrap(), &pulse()).unwrap();
    assert_eq!((on_disk.min, on_disk.max), (-2.0, 4.0));
    let wesl =
        std::fs::read_to_string(dir.path().join("assets/shaders/autogen/curves/dir_common/crv_pulse.wesl")).unwrap();
    assert!(wesl.contains("vec3<f32>(0.2, 4.0, "), "{wesl}");
}

#[test]
fn new_and_renamed_curves_follow_their_users() {
    let (dir, mut e) = project();
    e.apply(Action::NewAssetDialog(NewKind::Curve));
    e.apply(Action::Submit("ramps/fade".into()));
    let fade = AssetId::new(AssetType::Curve, "ramps/fade");
    assert_eq!(e.curve_index(&fade), e.active);
    assert!(dir.path().join("assets/shaders/autogen/curves/dir_ramps/crv_fade.wesl").is_file());
    assert!(e.curve_paths().contains(&"ramps/fade".to_string()));

    // A material using `common/pulse` is open; renaming the curve updates the node, the file and the tab.
    let bands = AssetId::new(AssetType::Material, "fx/curve_bands");
    e.apply(Action::OpenAsset(bands.clone()));
    e.apply(Action::OpenAsset(pulse()));
    e.apply(Action::RenameDialog(pulse()));
    e.apply(Action::Submit("ramps/pulse".into()));
    let moved = AssetId::new(AssetType::Curve, "ramps/pulse");
    assert!(e.curve_index(&moved).is_some());
    let d = e.tabs[e.doc_index(&bands).unwrap()].graph().unwrap();
    assert!(!d.is_dirty());
    let refs = shader_graph_core::registry::curve_refs(&d.asset, &e.registry);
    assert!(refs.iter().any(|r| r.2 == "ramps/pulse"), "{refs:?}");
    let node = refs.iter().find(|r| r.2 == "ramps/pulse").unwrap().0.clone();
    assert_eq!(crate::canvas::node_title(&e, &d.asset, &node), "pulse (Sample Curve)");
    e.validate();
    assert!(!e.all_diagnostics().iter().any(Diagnostic::is_error), "{:?}", e.all_diagnostics());
}

#[test]
fn curve_tabs_are_restored_on_next_launch() {
    let (dir, mut e) = project();
    let config = dir.path().join("config/config.ron");
    e.config_path = Some(config.clone());
    e.apply(Action::OpenAsset(pulse()));
    e.apply(Action::Quit);
    let mut next = Editor::with_config(UserConfig::load(&config), Some(config));
    next.open_project(dir.path()).unwrap();
    assert_eq!(next.curve_doc().map(|c| &c.id), Some(&pulse()));
}

#[test]
fn curve_review_regressions() {
    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(pulse()));
    let start = points(&e);

    // Typing a T that moves the point past a neighbour rebinds the fields to its new index.
    e.apply(Action::Curve(CurveAction::Select([1].into())));
    let props = e.ui.props;
    e.apply(Action::SetComponent { target: PropTarget::CurvePoint(1), comp: 0, value: 0.6, is_final: true });
    assert_eq!(e.curve_doc().unwrap().selection, [2].into());
    assert!(e.ui.props > props);
    assert_eq!(points(&e)[2], (0.6, 1.0));
    e.apply(Action::SetComponent { target: PropTarget::CurvePoint(2), comp: 1, value: 0.9, is_final: true });
    assert_eq!(points(&e)[2], (0.6, 0.9));
    // Two typed edits, two undo steps.
    e.apply(Action::Undo);
    assert_eq!(points(&e)[2], (0.6, 1.0));
    e.apply(Action::Undo);
    assert_eq!(points(&e), start);

    // A clamped (no-op) drag does not glue the next real drag to the previous step.
    e.apply(Action::Curve(CurveAction::Add((0.9, 0.5))));
    let added = points(&e);
    let last = added.len() - 1;
    let drag = |e: &mut Editor, delta| {
        let (points, selection) = CurveDoc::moved(&added, &[last].into(), delta);
        e.apply(Action::Curve(CurveAction::SetPoints { points, selection, merge: true }));
    };
    drag(&mut e, (0.5, 0.5));
    drag(&mut e, (0.0, -0.5));
    e.apply(Action::CommitInline);
    e.apply(Action::Undo);
    assert_eq!(points(&e), added);

    // Paste and Ctrl+D place copies beside the originals; Ctrl+D keeps the clipboard.
    e.apply(Action::Curve(CurveAction::Select([2].into())));
    e.apply(Action::Copy);
    let copied = e.curve_clipboard.clone();
    e.apply(Action::Paste(None));
    e.apply(Action::Duplicate);
    assert_eq!(e.curve_clipboard, copied);
    let p = points(&e);
    assert_eq!(p.len(), added.len() + 2);
    let (t, v) = added[2];
    assert!(
        p.iter()
            .filter(|q| q.1 == v)
            .all(|q| q.0 == t || (q.0 - t - 0.05).abs() < 1e-6 || (q.0 - t - 0.1).abs() < 1e-6)
    );
    assert_eq!(p.iter().filter(|q| **q == (t, v)).count(), 1, "no copy on top of the original");
}

fn tuning() -> AssetId {
    AssetId::new(AssetType::Consts, "common/tuning")
}

fn global_tint() -> AssetId {
    AssetId::new(AssetType::Material, "fx/global_tint")
}

fn set(e: &mut Editor, target: PropTarget, value: Value) {
    e.apply(Action::SetValue { target, value, merge: false });
}

/// The OK action of the open confirmation dialog.
fn confirm(e: &Editor) -> Action {
    match &e.dialog {
        Some(Dialog::Confirm { buttons, .. }) => buttons.last().unwrap().1.clone(),
        other => panic!("no confirmation: {other:?}"),
    }
}

#[test]
fn consts_tab_adds_edits_and_saves() {
    let (dir, mut e) = project();
    e.apply(Action::NewAssetDialog(NewKind::Consts));
    e.apply(Action::Submit("more/extra".into()));
    let extra = AssetId::new(AssetType::Consts, "more/extra");
    assert_eq!(e.consts_index(&extra), e.active);
    assert!(e.doc().is_none(), "graph commands must not reach a table tab");
    assert!(e.consts_doc().unwrap().table.consts.is_empty());

    // The add form refuses bad and taken names (also from other tables).
    for (name, problem) in [("Wind", "must"), ("glow_amount", "common/tuning")] {
        set(&mut e, PropTarget::ConstDraftName, Value::Str(name.into()));
        e.apply(Action::Consts(ConstsAction::Add));
        let err = e.consts_doc().unwrap().form_error.clone().unwrap_or_default();
        assert!(err.contains(problem), "{name}: {err}");
    }
    set(&mut e, PropTarget::ConstDraftName, Value::Str("wind".into()));
    set(&mut e, PropTarget::ConstDraftType, Value::Str("vec3".into()));
    e.apply(Action::Consts(ConstsAction::Add));
    let c = e.consts_doc().unwrap();
    assert_eq!((c.form_error.as_deref(), c.draft_name.as_str()), (None, ""));
    assert_eq!(c.table.get("wind").unwrap().ty, ConstType::Vec3);

    // A picker drag (all components, merged) is one undo step; a typed component another.
    let target = PropTarget::Const("wind".into());
    for x in [0.2, 0.4] {
        for (comp, value) in [x, 0.5, 0.6, 1.0].into_iter().enumerate() {
            e.apply(Action::SetComponent { target: target.clone(), comp, value, is_final: false });
        }
    }
    e.apply(Action::CommitInline);
    e.apply(Action::SetComponent { target: target.clone(), comp: 2, value: 3.0, is_final: true });
    assert_eq!(e.current_value(&target), Some(Value::Float3([0.4, 0.5, 3.0])), "values beyond 0..1 are fine");
    e.apply(Action::Undo);
    assert_eq!(e.current_value(&target), Some(Value::Float3([0.4, 0.5, 0.6])));
    e.apply(Action::Undo);
    assert_eq!(e.current_value(&target), Some(Value::Float3([1.0; 3])), "new constants start at 1.0");
    e.apply(Action::Redo);

    // Without readers a type change needs no confirmation (and converts the value).
    set(&mut e, PropTarget::ConstType("wind".into()), Value::Str("f32".into()));
    assert!(e.dialog.is_none());
    assert_eq!(e.current_value(&target), Some(Value::Float(0.4)));
    e.apply(Action::Save);
    let on_disk = assets::load_consts(e.project.as_ref().unwrap(), &extra).unwrap();
    assert_eq!(on_disk.get("wind").unwrap().value, Value::Float(0.4));

    // Editing a table regenerates the shaders reading it; unsaved values already show in the editor.
    e.apply(Action::OpenAsset(tuning()));
    let amount = PropTarget::Const("glow_amount".into());
    e.apply(Action::SetComponent { target: amount, comp: 0, value: 0.9, is_final: true });
    assert_eq!(e.global_consts()["glow_amount"].1.value, Value::Float(0.9));
    e.apply(Action::Save);
    let wesl = std::fs::read_to_string(dir.path().join("assets/shaders/autogen/dir_fx/mat_global_tint.wesl")).unwrap();
    assert!(wesl.contains("0.9f"), "{wesl}");
    assert!(!dir.path().join("assets/shaders/autogen/dir_common").exists());
}

#[test]
fn breaking_const_edits_ask_and_clear_references() {
    let (_dir, mut e) = project();
    let usages = |e: &Editor, name: &str| e.const_usages(&[name.to_string()]);
    e.apply(Action::OpenAsset(global_tint()));
    e.apply(Action::OpenAsset(tuning()));
    assert_eq!(usages(&e, "glow_amount"), vec![(global_tint(), 1)]);

    // A type change of a constant in use waits for the confirmation.
    let ty = PropTarget::ConstType("glow_amount".into());
    set(&mut e, ty.clone(), Value::Str("vec2".into()));
    assert_eq!(e.current_value(&ty), Some(Value::Str("f32".into())), "not changed before OK");
    e.apply(Action::CloseDialog);
    set(&mut e, ty.clone(), Value::Str("vec2".into()));
    e.apply(confirm(&e));
    assert!(e.dialog.is_none());
    assert_eq!(e.current_value(&ty), Some(Value::Str("vec2".into())));
    // OK saves the table with the cleared references.
    assert!(!e.is_dirty());
    let table = assets::load_consts(e.project.as_ref().unwrap(), &tuning()).unwrap();
    assert_eq!(table.get("glow_amount").unwrap().ty, ConstType::Vec2);
    // The open graph had no unsaved changes: edited and saved right away.
    let g = e.doc_index(&global_tint()).unwrap();
    let d = e.tabs[g].graph().unwrap();
    assert!(!d.is_dirty());
    assert!(const_refs(&d.asset, &e.registry).iter().all(|r| r.2 != "glow_amount"));
    let on_disk = assets::load(e.project.as_ref().unwrap(), &global_tint()).unwrap();
    assert!(const_refs(&on_disk, &e.registry).iter().all(|r| r.2 != "glow_amount"));
    assert!(usages(&e, "glow_amount").is_empty());
    e.validate();
    let codes: Vec<Code> = e.diags[&global_tint()].iter().map(|d| d.code).collect();
    assert_eq!(codes, vec![Code::W106]);

    // Undo / redo of a type change asks too once a node reads the constant again.
    let t = e.consts_index(&tuning()).unwrap();
    e.apply(Action::ActivateTab(g));
    let node = e.doc().unwrap().asset.nodes.iter().find(|(_, n)| n.kind == "stdlib::GlobalConstF32").unwrap();
    let param = PropTarget::Param { node: node.0.clone(), key: "const".into() };
    e.apply(Action::Undo); // the graph's clearing
    assert_eq!(e.current_value(&param), Some(Value::Str("glow_amount".into())));
    e.apply(Action::ActivateTab(t));
    e.apply(Action::Undo);
    assert!(e.dialog.is_some(), "undoing the type change breaks the reader");
    assert_eq!(e.current_value(&ty), Some(Value::Str("vec2".into())));
    e.apply(confirm(&e));
    assert_eq!(e.current_value(&ty), Some(Value::Str("f32".into())));
    assert!(usages(&e, "glow_amount").is_empty());

    // Deleting a constant read by a closed graph rewrites that file.
    let g = e.doc_index(&global_tint()).unwrap();
    e.apply(Action::CloseTab(g));
    e.apply(Action::Consts(ConstsAction::Delete { name: "fancy_stripes".into(), confirmed: false }));
    assert!(e.consts_doc().unwrap().table.get("fancy_stripes").is_some());
    e.apply(confirm(&e));
    assert!(e.consts_doc().unwrap().table.get("fancy_stripes").is_none());
    let on_disk = assets::load(e.project.as_ref().unwrap(), &global_tint()).unwrap();
    assert!(const_refs(&on_disk, &e.registry).iter().all(|r| r.2 != "fancy_stripes"));
    // An unused constant goes without asking (undo and delete again).
    e.apply(Action::Undo);
    assert!(e.dialog.is_none());
    e.apply(Action::Consts(ConstsAction::Delete { name: "fancy_stripes".into(), confirmed: false }));
    assert!(e.dialog.is_none());
    assert!(e.consts_doc().unwrap().table.get("fancy_stripes").is_none());
}

#[test]
fn deleting_a_table_clears_the_references_to_its_constants() {
    let (_dir, mut e) = project();
    e.apply(Action::DeleteDialog(tuning()));
    let Some(Dialog::Confirm { message, .. }) = &e.dialog else { panic!("no dialog") };
    assert!(message.contains("fx/global_tint.material.ron (6 nodes)"), "{message}");
    assert!(!message.contains("generated files"), "{message}");
    e.apply(confirm(&e));
    assert!(!tuning().file(e.project.as_ref().unwrap()).exists());
    let on_disk = assets::load(e.project.as_ref().unwrap(), &global_tint()).unwrap();
    assert!(const_refs(&on_disk, &e.registry).is_empty());
    e.validate();
    assert!(e.diags[&global_tint()].iter().all(|d| d.code == Code::W106));
}

#[test]
fn consts_tabs_are_restored_and_follow_renames() {
    let (dir, mut e) = project();
    e.apply(Action::OpenAsset(tuning()));
    e.rename(&tuning(), "settings/tuning");
    let moved = AssetId::new(AssetType::Consts, "settings/tuning");
    assert_eq!(e.consts_doc().map(|c| &c.id), Some(&moved));
    assert!(e.diags.values().flatten().all(|d| !d.is_error()), "names, not paths: users are unaffected");
    e.save_tabs_to_config();
    let config = e.config.clone();
    let mut next = Editor::with_config(config, None);
    next.open_project(dir.path()).unwrap();
    assert_eq!(next.consts_doc().map(|c| &c.id), Some(&moved));
}

#[test]
fn tab_doc_dispatch_matches_kinds() {
    use crate::tab::TabKey;
    let (_dir, mut e) = project();
    e.regenerate();
    let rock = AssetId::new(AssetType::Material, "env/rock_wet");
    let pulse = AssetId::new(AssetType::Curve, "common/pulse");
    let tuning = AssetId::new(AssetType::Consts, "common/tuning");
    e.apply(Action::OpenAsset(rock.clone()));
    e.apply(Action::ShowGenerated(rock.clone(), Lang::Wesl));
    e.apply(Action::OpenAsset(pulse.clone()));
    e.apply(Action::OpenAsset(tuning.clone()));
    let expected = [
        (ViewKind::Graph, "shaders/env/rock_wet.material", false, Some(&rock), TabKey::Asset(rock.clone())),
        (
            ViewKind::Text,
            "assets/shaders/autogen/dir_env/mat_rock_wet.wesl",
            true,
            None,
            TabKey::Generated(rock.clone(), Lang::Wesl),
        ),
        (ViewKind::Curve, "shaders/common/pulse.curve", false, Some(&pulse), TabKey::Asset(pulse.clone())),
        (ViewKind::Consts, "shaders/common/tuning.consts", false, Some(&tuning), TabKey::Asset(tuning.clone())),
    ];
    assert_eq!(e.tabs.len(), expected.len());
    for (tab, (view, title, read_only, asset, key)) in e.tabs.iter().zip(expected) {
        let doc = tab.doc();
        assert_eq!(doc.view(), view, "{title}");
        assert_eq!(doc.base_title(), title);
        assert_eq!(doc.is_read_only(), read_only, "{title}");
        assert_eq!(doc.asset(), asset, "{title}");
        assert!(!doc.is_dirty(), "{title}");
        let owner = match &key {
            TabKey::Asset(id) | TabKey::Generated(id, _) => Some(id),
            TabKey::GameImage(_) | TabKey::Internal(_) => None,
        };
        assert_eq!(tab.owner(), owner, "{title}");
        let config_key = doc.config_key().unwrap();
        assert_eq!(parse_config_key(&config_key), Some(key), "{config_key}");
    }
    // Only editable documents are found by asset; the generated file's tab belongs to its source.
    assert_eq!(e.asset_index(&rock), Some(0));
    assert_eq!(e.asset_index(&pulse), Some(2));
    assert_eq!(e.asset_index(&tuning), Some(3));
}

/// The editor statusbar's data (design §2.3): unsaved marker, this asset's errors / warnings, a node count and the
/// generated WESL's line count.
#[test]
fn status_of_graph_counts_nodes_and_diagnostics() {
    let (dir, mut e) = project();
    let rock = AssetId::new(AssetType::Material, "env/rock_wet");
    e.apply(Action::OpenAsset(rock.clone()));
    let status = e.status_info().expect("a graph tab has a statusbar");
    assert!(!status.dirty);
    let wesl_lines = |n| crate::ui::plural(n, "WESL line", "WESL lines");
    let any_wesl = |c: &String| c.ends_with(" WESL line") || c.ends_with(" WESL lines");
    assert!(!status.counts.iter().any(any_wesl), "not generated yet: {:?}", status.counts);
    // An unconnected node: a warning of this asset.
    e.apply(Action::AddNode { kind: "stdlib::Time".into(), pos: (0.0, 0.0), link: None, params: Vec::new() });
    e.validate();
    let status = e.status_info().unwrap();
    assert!(status.dirty);
    assert_eq!(status.counts[0], crate::ui::plural(e.doc().unwrap().asset.nodes.len(), "node", "nodes"));
    let own: Vec<&Diagnostic> = e.diags.get(&rock).into_iter().flatten().collect();
    assert_eq!(status.errors, own.iter().filter(|d| d.is_error()).count());
    assert_eq!(status.warnings, own.iter().filter(|d| !d.is_error()).count());
    assert!(status.warnings > 0, "{own:?}");

    e.apply(Action::RegenAll);
    let wesl = std::fs::read_to_string(dir.path().join("assets/shaders/autogen/dir_env/mat_rock_wet.wesl")).unwrap();
    let status = e.status_info().unwrap();
    assert!(status.counts.contains(&wesl_lines(wesl.lines().count())), "{:?}", status.counts);

    // A preview writes the asset's WESL from memory (a scoped regeneration): the count follows the file.
    let lerp = add_lerp(&mut e);
    e.apply(Action::Select([lerp].into()));
    press_v(&mut e);
    let previewed = std::fs::read_to_string(rock_wet_wesl(&e)).unwrap();
    assert_ne!(previewed.lines().count(), wesl.lines().count());
    let lines = wesl_lines(previewed.lines().count());
    assert!(e.status_info().unwrap().counts.contains(&lines), "{:?}", e.status_info().unwrap().counts);
    e.apply(Action::DiscardTab(e.active.unwrap()));

    // Known for a tab opened later (the file is read once) and refreshed with a generated text tab.
    let mut next = Editor::default();
    next.open_project(dir.path()).unwrap();
    next.apply(Action::OpenAsset(rock.clone()));
    assert!(next.status_info().unwrap().counts.contains(&wesl_lines(wesl.lines().count())));
    let path = dir.path().join("assets/shaders/autogen/dir_env/mat_rock_wet.wesl");
    std::fs::write(&path, format!("{wesl}// one more\n")).unwrap();
    next.apply(Action::ShowGenerated(rock.clone(), Lang::Wesl));
    next.apply(Action::ActivateTab(0));
    let lines = wesl_lines(wesl.lines().count() + 1);
    assert!(next.status_info().unwrap().counts.contains(&lines), "{:?}", next.status_info().unwrap().counts);
}

#[test]
fn status_of_curve_and_consts() {
    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(pulse()));
    let points = e.curve_doc().unwrap().curve.points.len();
    let status = e.status_info().expect("a curve tab has a statusbar");
    assert_eq!(status.counts, vec![crate::ui::plural(points, "point", "points")]);
    assert!(!status.dirty);
    e.apply(Action::OpenAsset(tuning()));
    let rows = e.consts_doc().unwrap().table.consts.len();
    let status = e.status_info().expect("a consts tab has a statusbar");
    assert_eq!(status.counts, vec![crate::ui::plural(rows, "constant", "constants")]);
}

#[test]
fn no_status_for_read_only_kinds() {
    let (dir, mut e) = project();
    e.regenerate();
    let rock = AssetId::new(AssetType::Material, "env/rock_wet");
    e.apply(Action::ShowGenerated(rock.clone(), Lang::Wesl));
    assert!(e.tab().and_then(Tab::text).is_some());
    assert_eq!(e.status_info(), None, "generated WESL");
    let path = "textures/env/rock.png";
    let file = dir.path().join("assets").join(path);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, b"not decoded by the editor state").unwrap();
    e.refresh_game_files();
    e.apply(Action::OpenGameImage(path.into()));
    assert!(e.tab().and_then(Tab::image).is_some());
    assert_eq!(e.status_info(), None, "game image");
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Texture, "textures/rock_albedo")));
    assert!(e.texture_doc().is_some());
    assert_eq!(e.status_info(), None, "texture");
    while !e.tabs.is_empty() {
        e.close_tab(e.tabs.len() - 1, true);
    }
    assert_eq!(e.active, None);
    assert_eq!(e.status_info(), None, "no tab");
}

#[test]
fn watermark_per_kind() {
    let (_dir, mut e) = project();
    e.regenerate();
    let rock = AssetId::new(AssetType::Material, "env/rock_wet");
    let mask = AssetId::new(AssetType::Function, "common/wet_mask");
    e.apply(Action::OpenAsset(rock.clone()));
    e.apply(Action::OpenAsset(mask.clone()));
    e.apply(Action::ShowGenerated(rock.clone(), Lang::Wesl));
    e.apply(Action::ShowGenerated(rock.clone(), Lang::Rust));
    e.apply(Action::OpenAsset(pulse()));
    e.apply(Action::OpenAsset(tuning()));
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Texture, "textures/rock_albedo")));
    let marks: Vec<_> = e.tabs.iter().map(|t| t.doc().watermark()).collect();
    let expected = ["pbr3d", "function", "wesl", "rust", "curve", "consts", "texture"];
    assert_eq!(marks, expected.map(Some));
}

#[test]
fn keep_mine_takes_the_file_as_saved_baseline() {
    let (_dir, mut e) = project();
    let project = e.project.clone().unwrap();
    let rock = AssetId::new(AssetType::Material, "env/rock_wet");
    let pulse = AssetId::new(AssetType::Curve, "common/pulse");
    e.apply(Action::OpenAsset(rock.clone()));
    e.apply(Action::AddNode { kind: "stdlib::Time".into(), pos: (0.0, 0.0), link: None, params: Vec::new() });
    e.apply(Action::OpenAsset(pulse.clone()));
    e.curve_doc_mut().unwrap().add_point((0.5, 0.5));
    // Both files change outside the editor.
    let mut asset = assets::load(&project, &rock).unwrap();
    asset.add_node(shader_graph_core::model::Node::new("stdlib::Sine", (0.0, 0.0)));
    ops::save(&project, &rock, &asset).unwrap();
    let mut curve = assets::load_curve(&project, &pulse).unwrap();
    curve.points.push((0.25, 0.75));
    curve.normalize();
    ops::save_curve(&project, &pulse, &curve).unwrap();
    let (rock_edit, pulse_edit) = (e.tabs[0].graph().unwrap().asset.clone(), e.curve_doc().unwrap().curve.clone());
    e.apply(Action::KeepMine(rock.clone()));
    e.apply(Action::KeepMine(pulse.clone()));
    let (doc, curve_doc) = (e.tabs[0].graph().unwrap(), e.curve_doc().unwrap());
    assert_eq!(doc.saved, asset.to_ron());
    assert_eq!(curve_doc.saved, curve.to_ron());
    assert_eq!((&doc.asset, &curve_doc.curve), (&rock_edit, &pulse_edit), "the edits stay");
    assert!(e.tabs.iter().all(|t| t.doc().is_dirty()));
}

#[test]
fn collapse_state_is_per_tree() {
    use crate::ui::tree::{ProjectTree, TreeItem, TreeSource};
    let (_dir, mut e) = project();
    let rock = TreeItem::Asset(AssetId::new(AssetType::Material, "env/rock_wet"));
    let rows = |e: &Editor| ProjectTree.rows(e).into_iter().map(|(_, item)| item).collect::<Vec<_>>();
    assert!(rows(&e).contains(&rock));

    e.apply(Action::ToggleFolder(TreeId::Project, "env".into()));
    assert!(e.is_collapsed(TreeId::Project, "env"));
    assert!(!e.is_collapsed(TreeId::GameAssets, "env"));
    assert!(!e.hidden_in(TreeId::GameAssets, "env/rock.png"));
    let collapsed = rows(&e);
    assert!(collapsed.contains(&TreeItem::Folder("env".into())));
    assert!(!collapsed.contains(&rock));

    // Collapsing the same path in the other tree leaves the Project tree alone.
    e.apply(Action::ToggleFolder(TreeId::GameAssets, "env".into()));
    e.apply(Action::ToggleFolder(TreeId::Project, "env".into()));
    assert!(e.is_collapsed(TreeId::GameAssets, "env"));
    assert!(!e.is_collapsed(TreeId::Project, "env"));
    assert!(rows(&e).contains(&rock));
}

#[test]
fn conversion_copies_params_and_generates_unique_name() {
    let (_dir, mut e) = project();
    let lerp = add_lerp(&mut e);
    let add = |e: &mut Editor, kind: &str, params: Vec<(&str, Value)>| {
        let before: Vec<_> = e.doc().unwrap().asset.nodes.keys().cloned().collect();
        let params = params.into_iter().map(|(k, v)| (k.to_string(), v)).collect();
        e.apply(Action::AddNode { kind: kind.into(), pos: (0.0, 0.0), link: None, params });
        e.doc().unwrap().asset.nodes.keys().find(|k| !before.contains(k)).unwrap().clone()
    };
    for n in ["x", "x_1"] {
        add(&mut e, "param::Scalar", vec![("name", Value::Str(n.into()))]);
    }
    let src = add(&mut e, "param::Scalar", vec![("name", Value::Str("old".into())), ("default", Value::Float(0.7))]);
    e.apply(Action::Connect { from: (src.clone(), "out".into()), to: (lerp.clone(), "a".into()) });
    let original = e.doc().unwrap().asset.nodes[&src].clone();
    let conv = Conversion {
        kind: "param::Color".into(),
        params: vec![("group".into(), "group".into())],
        unique_name: Some(("name".into(), "x".into())),
        pins: vec![("out".into(), "r".into())],
    };
    e.apply(Action::SetValue {
        target: PropTarget::Param { node: src.clone(), key: "group".into() },
        value: Value::Str("g".into()),
        merge: false,
    });
    let original =
        shader_graph_core::model::Node { params: e.doc().unwrap().asset.nodes[&src].params.clone(), ..original };
    e.apply(Action::ConvertNodes { nodes: vec![src.clone()], conv });
    let d = e.doc().unwrap();
    let n = &d.asset.nodes[&src];
    assert_eq!(n.kind, "param::Color");
    assert_eq!(n.params["name"], Value::Str("x_2".into()));
    assert_eq!(n.params["group"], Value::Str("g".into()), "a mapped param is carried");
    assert!(!n.params.contains_key("default"), "unlisted params are dropped");
    let edge = d.asset.edge_into(&lerp, "a").expect("the edge on the renamed pin survives");
    assert_eq!(edge.from, (src.clone(), "r".to_string()));
    e.apply(Action::Undo);
    assert_eq!(e.doc().unwrap().asset.nodes[&src], original);
    assert_eq!(e.doc().unwrap().asset.edge_into(&lerp, "a").unwrap().from.1, "out");
}

/// Texture assets (design §1.1) are loaded and validated with the project; they open in a Texture tab (a file that
/// does not load reports why and opens nothing), which follows a rename.
#[test]
fn texture_asset_is_loaded_validated_and_opened() {
    let (dir, _) = project();
    let p = Project::open(dir.path()).unwrap();
    let id = AssetId::new(AssetType::Texture, "textures/rock");
    shader_project::ops::create_texture(&p, &id, &shader_graph_core::TextureAsset::new("textures/rock.png")).unwrap();
    std::fs::write(dir.path().join("shaders/textures/broken.texture.ron"), "TextureAsset(").unwrap();
    let mut e = Editor::default();
    e.open_project(dir.path()).unwrap();
    assert!(e.disk.contains(&id));
    e.validate();
    assert!(e.all_diagnostics().iter().any(|d| d.code == Code::W109), "{:?}", e.all_diagnostics());
    let tabs = e.tabs.len();
    e.apply(Action::OpenAsset(id.clone()));
    assert_eq!(e.tabs.len(), tabs + 1);
    assert_eq!(texture_doc(&e).asset.path, "textures/rock.png");
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Texture, "textures/broken")));
    assert_eq!(e.tabs.len(), tabs + 1);
    assert!(e.messages.last().is_some_and(|m| m.level == Level::Error && m.text.contains("broken")));
    e.apply(Action::CloseDialog);

    e.apply(Action::ActivateTab(tabs));
    e.apply(Action::RenameDialog(id.clone()));
    e.apply(Action::Submit("textures/stone".into()));
    let moved = AssetId::new(AssetType::Texture, "textures/stone");
    assert_eq!(texture_doc(&e).id, moved);
    assert_eq!(e.tab().unwrap().title(), "shaders/textures/stone.texture");
    assert!(!e.is_dirty());
}

/// Review focus 5 in the editor: open graphs follow a renamed / deleted Texture asset (design §1.2) like they follow
/// curves — a clean document reloads the rewritten file, a dirty one gets the change as an unsaved edit.
#[test]
fn open_documents_follow_texture_rename_and_delete() {
    use shader_graph_core::registry::texture_refs;
    let (dir, _) = project();
    let p = Project::open(dir.path()).unwrap();
    let tex = AssetId::new(AssetType::Texture, "textures/rock");
    ops::create_texture(&p, &tex, &shader_graph_core::TextureAsset::new("textures/rock.png")).unwrap();
    let rock = AssetId::new(AssetType::Material, "env/rock_wet");
    let mut a = assets::load(&p, &rock).unwrap();
    for n in a.nodes.values_mut().filter(|n| n.kind == "param::TextureSample2D") {
        n.params.insert("default".into(), Value::Str("textures/rock".into()));
    }
    ops::save(&p, &rock, &a).unwrap();
    let mut e = Editor::default();
    e.open_project(dir.path()).unwrap();
    e.apply(Action::OpenAsset(rock.clone()));
    let doc = |e: &Editor| e.tabs[e.doc_index(&rock).unwrap()].graph().unwrap().clone();
    let ids = |a: &Asset, e: &Editor| texture_refs(a, &e.registry).into_iter().map(|r| r.2).collect::<Vec<_>>();
    let node = texture_refs(&doc(&e).asset, &e.registry)[0].0.clone();
    assert_eq!(e.referenced_asset(&node), Some(tex.clone()));

    e.apply(Action::RenameDialog(tex.clone()));
    e.apply(Action::Submit("textures/stone".into()));
    let stone = AssetId::new(AssetType::Texture, "textures/stone");
    assert!(e.disk.contains(&stone));
    assert_eq!(ids(&doc(&e).asset, &e), vec!["textures/stone"; 2]);
    assert!(!doc(&e).is_dirty());
    assert_eq!(e.referenced_asset(&node), Some(stone.clone()));

    let i = e.doc_index(&rock).unwrap();
    e.tabs[i].graph_mut().unwrap().edit(|a| a.nodes.get_mut(&node).unwrap().pos.0 += 10.0);
    e.apply(Action::Delete(stone.clone()));
    assert!(!e.disk.contains(&stone));
    assert!(ids(&doc(&e).asset, &e).is_empty());
    assert!(doc(&e).is_dirty(), "unsaved edits stay unsaved");
    assert!(ids(&assets::load(&p, &rock).unwrap(), &e).is_empty(), "the file is cleared too");
    assert_eq!(e.referenced_asset(&node), None);
}

/// The Game Assets tree (design §4.2) lists the images under `assets/` (not `assets/shaders/`) and the folders that
/// hold them; it is read-only: its rows offer no rename / move / delete and nothing can be dropped into it. RMB on a
/// file creates its Texture asset (disabled once one exists).
#[test]
fn game_assets_tree_rows() {
    use crate::ui::tree::{self, GameAssetsTree, ProjectTree, TreeItem, TreeRow, TreeSource};
    let (dir, mut e) = project();
    for f in ["textures/env/rock.png", "textures/env/notes.txt", "docs/readme.md", "top.KTX2", "shaders/internal/x.png"]
    {
        let p = dir.path().join("assets").join(f);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, b"x").unwrap();
    }
    e.refresh_game_files();
    let folder = |f: &str| TreeItem::Folder(f.into());
    let file = |f: &str| TreeItem::GameFile(f.into());
    assert_eq!(
        GameAssetsTree.rows(&e),
        vec![
            (0, folder("")),
            (1, folder("textures")),
            (2, folder("textures/env")),
            (3, file("textures/env/rock.png")),
            (1, file("top.KTX2")),
        ]
    );
    assert!(tree::source(TreeId::GameAssets).is_some_and(|s| s.read_only()));

    let row = |tree, item| TreeRow { tree, item };
    let rock = row(TreeId::GameAssets, file("textures/env/rock.png"));
    let env = row(TreeId::GameAssets, folder("textures/env"));
    let destructive = |a: &Action| {
        matches!(
            a,
            Action::RenameDialog(_)
                | Action::MoveDialog(_)
                | Action::DeleteDialog(_)
                | Action::DeleteFolderDialog(_)
                | Action::MoveTo(..)
                | Action::NewAssetDialog(_)
                | Action::NewFolderDialog
        )
    };
    let menu = tree::row_menu(&e, &rock);
    assert_eq!(menu.iter().map(|m| m.label.as_str()).collect::<Vec<_>>(), ["Create Texture Asset"]);
    assert!(menu[0].disabled.is_none());
    assert!(!menu.iter().any(|m| destructive(&m.action)));
    assert!(tree::row_menu(&e, &env).is_empty());
    assert!(tree::row_menu(&e, &row(TreeId::GameAssets, folder(""))).is_empty());
    assert_eq!(tree::click_actions(&env, 1), vec![Action::ToggleFolder(TreeId::GameAssets, "textures/env".into())]);
    assert_eq!(tree::click_actions(&rock, 2), vec![Action::OpenGameImage("textures/env/rock.png".into())]);
    assert_eq!(tree::drop_action(&env, &rock), None);
    let material = row(TreeId::Project, TreeItem::Asset(AssetId::new(AssetType::Material, "env/rock_wet")));
    assert_eq!(tree::drop_action(&env, &material), None);
    assert_eq!(tree::drop_action(&row(TreeId::Project, folder("fx")), &rock), None);

    // "Create Texture Asset": `shaders/textures/env/rock.texture.ron`, shown in the Project tree.
    let id = AssetId::new(AssetType::Texture, "textures/env/rock");
    e.apply(menu[0].action.clone());
    assert!(id.file(e.project.as_ref().unwrap()).is_file());
    assert!(ProjectTree.rows(&e).iter().any(|(_, item)| *item == TreeItem::Asset(id.clone())));
    let again = tree::row_menu(&e, &rock);
    assert!(again[0].disabled.as_deref().is_some_and(|t| t.contains("textures/env/rock")), "{:?}", again[0].disabled);
    // Applied anyway (a stale menu): refused, nothing new.
    let count = e.disk.textures.len();
    e.apply(menu[0].action.clone());
    assert_eq!(e.disk.textures.len(), count);
}

/// The Project tree ends with the virtual read-only `Internal` folder of built-in textures (design §2).
#[test]
fn internal_folder_lists_internal_textures() {
    use crate::ui::tree::{self, INTERNAL_FOLDER, ProjectTree, TreeItem, TreeRow, TreeSource};
    use shader_stdlib::internal::DEFAULT_TEXTURE;
    let (_dir, mut e) = project();
    let internal = TreeItem::Folder(INTERNAL_FOLDER.into());
    let default = TreeItem::Internal(DEFAULT_TEXTURE.into());
    let rows = ProjectTree.rows(&e);
    assert_eq!(rows[rows.len() - 2..], [(1, internal.clone()), (2, default.clone())]);
    assert_eq!(tree::row_label(TreeId::Project, &internal), "Internal");
    assert_eq!(tree::row_label(TreeId::Project, &default), "default");

    let row = |item| TreeRow { tree: TreeId::Project, item };
    assert!(tree::row_menu(&e, &row(internal.clone())).is_empty());
    assert!(tree::row_menu(&e, &row(default.clone())).is_empty());
    // Toggles, but is never the target of "New ...".
    assert_eq!(
        tree::click_actions(&row(internal.clone()), 1),
        vec![Action::ToggleFolder(TreeId::Project, INTERNAL_FOLDER.into())]
    );
    let material = row(TreeItem::Asset(AssetId::new(AssetType::Material, "env/rock_wet")));
    assert_eq!(tree::drop_action(&row(internal.clone()), &material), None);
    assert_eq!(tree::drop_action(&row(TreeItem::Folder("fx".into())), &row(default.clone())), None);
    // Real folders still take drops.
    assert!(tree::drop_action(&row(TreeItem::Folder("fx".into())), &material).is_some());

    e.apply(Action::ToggleFolder(TreeId::Project, INTERNAL_FOLDER.into()));
    assert!(!ProjectTree.rows(&e).iter().any(|(_, item)| *item == default));
}

/// Game folder names may contain characters sorting before `/` (`-`, `.`, space): a folder's subfolders still come
/// right after it (and its files), not after its siblings.
#[test]
fn game_assets_tree_nests_folders_by_component() {
    use crate::ui::tree::{GameAssetsTree, TreeItem, TreeSource};
    let (dir, mut e) = project();
    for f in ["a/x.png", "a-b/y.png", "a/c/z.png", "a b/w.png"] {
        let p = dir.path().join("assets").join(f);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, b"x").unwrap();
    }
    e.refresh_game_files();
    let folder = |f: &str| TreeItem::Folder(f.into());
    let file = |f: &str| TreeItem::GameFile(f.into());
    assert_eq!(
        GameAssetsTree.rows(&e),
        vec![
            (0, folder("")),
            (1, folder("a")),
            (2, file("a/x.png")),
            (2, folder("a/c")),
            (3, file("a/c/z.png")),
            (1, folder("a b")),
            (2, file("a b/w.png")),
            (1, folder("a-b")),
            (2, file("a-b/y.png")),
        ]
    );
}

/// Double-clicking a game image opens it in a read-only image tab (design §4.4): once per file, remembered in the
/// user config as `@game/<path>`.
#[test]
fn open_game_image_tab() {
    use crate::tab::{TabKey, parse_config_key};
    use crate::ui::tree::{self, TreeItem, TreeRow};
    let (dir, mut e) = project();
    let path = "textures/env/rock.png";
    let file = dir.path().join("assets").join(path);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, b"not decoded by the editor state").unwrap();
    e.refresh_game_files();

    let row = TreeRow { tree: TreeId::GameAssets, item: TreeItem::GameFile(path.into()) };
    assert!(tree::click_actions(&row, 1).is_empty());
    let open = tree::click_actions(&row, 2);
    assert_eq!(open, vec![Action::OpenGameImage(path.into())]);

    let rock = AssetId::new(AssetType::Material, "env/rock_wet");
    e.apply(Action::OpenAsset(rock.clone()));
    for a in open.clone() {
        e.apply(a);
    }
    assert_eq!(e.tabs.len(), 2);
    assert_eq!(e.active, Some(1));
    let Some(Tab::Image(img)) = e.tab() else { panic!("image tab expected") };
    assert_eq!(img.path, path);
    let doc = e.tab().unwrap().doc();
    assert_eq!(doc.view(), ViewKind::Image);
    assert!(doc.is_read_only());
    assert!(!doc.is_dirty());
    assert_eq!(doc.asset(), None);
    assert_eq!(e.tab().unwrap().owner(), None);
    assert_eq!(doc.base_title(), "assets/textures/env/rock.png");
    let key = doc.config_key().unwrap();
    assert_eq!(key, "@game/textures/env/rock.png");
    assert_eq!(parse_config_key(&key), Some(TabKey::GameImage(path.into())));
    assert!(e.doc().is_none(), "no graph commands on an image tab");

    // Opening it again activates the same tab.
    e.apply(Action::OpenAsset(rock.clone()));
    assert_eq!(e.active, Some(0));
    e.apply(Action::OpenGameImage(path.into()));
    assert_eq!(e.tabs.len(), 2);
    assert_eq!(e.active, Some(1));

    // A changed file bumps the revision: the view decodes it again.
    let revision = e.tab().unwrap().doc().revision();
    std::fs::write(&file, b"changed, and longer than before").unwrap();
    e.reload_read_only_tabs();
    assert!(e.tab().unwrap().doc().revision() > revision);

    // Restored on the next launch.
    e.save_tabs_to_config();
    assert_eq!(e.config.tabs.values().next().unwrap().open[1], "@game/textures/env/rock.png");
    let config = e.config.clone();
    let mut next = Editor::with_config(config.clone(), None);
    next.open_project(dir.path()).unwrap();
    assert_eq!(next.tabs.len(), 2);
    assert_eq!(next.active, Some(1));
    assert!(matches!(next.tab(), Some(Tab::Image(i)) if i.path == path));

    // A file that is gone is not restored; opening a missing file only warns.
    std::fs::remove_file(&file).unwrap();
    let mut gone = Editor::with_config(config, None);
    gone.open_project(dir.path()).unwrap();
    assert_eq!(gone.tabs.len(), 1);
    gone.apply(Action::OpenGameImage(path.into()));
    assert_eq!(gone.tabs.len(), 1);
    assert!(gone.messages.last().is_some_and(|m| m.text.contains(path)));

    // Paths leaving `assets/` (or not naming a ktx2 / png) open nothing, even when the file exists.
    let outside = dir.path().join("outside.png");
    std::fs::write(&outside, b"x").unwrap();
    std::fs::write(dir.path().join("assets/notes.txt"), b"x").unwrap();
    let absolute = outside.to_string_lossy().replace('\\', "/");
    for bad in ["../outside.png", "textures/../../outside.png", absolute.as_str(), "notes.txt"] {
        gone.apply(Action::OpenGameImage(bad.into()));
        assert_eq!(gone.tabs.len(), 1, "{bad}");
        assert!(gone.messages.last().is_some_and(|m| m.text.starts_with("cannot open game image: ")), "{bad}");
    }
}

fn texture_doc(e: &Editor) -> &crate::texture_doc::TextureDoc {
    match e.tab() {
        Some(Tab::Texture(t)) => t,
        _ => panic!("texture tab expected"),
    }
}

/// The Texture tab (design §4.4): settings edits are undoable steps of the document, validation and saving see them.
#[test]
fn texture_tab_edit_undo_redo() {
    use crate::tab::TabKey;
    use crate::texture_doc::TextureField;
    use shader_graph_core::texture::{FilterMode, SamplerSettings, TextureSettings};
    let (_dir, mut e) = project();
    let project = e.project.clone().unwrap();
    let id = AssetId::new(AssetType::Texture, "textures/rock_albedo");
    e.apply(Action::OpenAsset(id.clone()));
    assert_eq!(e.tabs.len(), 1, "Texture assets open in a tab");
    let doc = e.tab().unwrap().doc();
    assert_eq!(doc.view(), ViewKind::Texture);
    assert_eq!(doc.asset(), Some(&id));
    assert_eq!(doc.watermark(), Some("texture"));
    assert_eq!(doc.base_title(), "shaders/textures/rock_albedo.texture");
    assert!(!doc.is_read_only() && !doc.is_dirty());
    assert_eq!(parse_config_key(&doc.config_key().unwrap()), Some(TabKey::Asset(id.clone())));
    assert_eq!(texture_doc(&e).asset.settings, TextureSettings::default());

    // Mag filter Nearest over the game's default sampler: a custom sampler.
    let custom = TextureSettings {
        sampler: Some(SamplerSettings { mag_filter: FilterMode::Nearest, ..SamplerSettings::image_plugin_default() }),
        ..TextureSettings::default()
    };
    e.apply(Action::Texture(TextureAction::SetSettings(custom.clone())));
    assert!(e.is_dirty());
    assert_eq!(texture_doc(&e).asset.settings, custom);
    assert_eq!(e.overridden_set().textures[&id].as_ref().unwrap().settings, custom, "validation sees the edit");
    e.apply(Action::Undo);
    assert!(!e.is_dirty());
    assert_eq!(texture_doc(&e).asset.settings, TextureSettings::default());
    e.apply(Action::Redo);
    assert_eq!(texture_doc(&e).asset.settings, custom);
    e.apply(Action::Undo);

    // The same through the form: "Default" sampler off, then the mag filter select.
    let field = |f| PropTarget::Texture(f);
    e.apply(Action::SetValue { target: field(TextureField::DefaultSampler), value: Value::Bool(false), merge: false });
    assert_eq!(texture_doc(&e).asset.settings.sampler, Some(SamplerSettings::image_plugin_default()));
    e.apply(Action::SetValue {
        target: field(TextureField::MagFilter),
        value: Value::Str("Nearest".into()),
        merge: false,
    });
    assert_eq!(texture_doc(&e).asset.settings, custom);
    assert_eq!(e.current_value(&field(TextureField::MagFilter)), Some(Value::Str("Nearest".into())));
    // A dragged number field is one step.
    for v in [4.0, 6.0, 8.0] {
        e.apply(Action::SetComponent { target: field(TextureField::LodMaxClamp), comp: 0, value: v, is_final: false });
    }
    e.apply(Action::SetComponent { target: field(TextureField::LodMaxClamp), comp: 0, value: 9.0, is_final: true });
    assert_eq!(texture_doc(&e).asset.settings.sampler.unwrap().lod_max_clamp, 9.0);
    e.apply(Action::Undo);
    assert_eq!(texture_doc(&e).asset.settings, custom);
    e.apply(Action::Undo);
    e.apply(Action::Undo);
    assert!(!e.is_dirty(), "back to the saved settings");
    e.apply(Action::Redo);
    e.apply(Action::Redo);

    // Saved like any document.
    e.apply(Action::Save);
    assert!(!e.is_dirty());
    assert_eq!(assets::load_texture(&project, &id).unwrap().settings, custom);

    // A sampler wgpu cannot create is kept as edited and reported (E013); Properties redraw with the problem.
    let aniso = TextureField::Anisotropy.set(&custom, &Value::Int(8));
    e.apply(Action::Texture(TextureAction::SetSettings(aniso.clone())));
    assert_eq!(texture_doc(&e).asset.settings, aniso);
    let props = e.ui.props;
    e.validate();
    assert!(e.diags[&id].iter().any(|d| d.code == Code::E013), "{:?}", e.diags[&id]);
    assert!(e.ui.props > props);
    let props = e.ui.props;
    e.validate();
    assert_eq!(e.ui.props, props, "unchanged problems: no redraw");
}

/// One game file, one Texture asset (design §1.1, review focus 1): a path edit, an undo or a "Create Texture Asset"
/// that would make two Texture assets point at one file is refused (open documents count as edited).
#[test]
fn texture_path_edit_refuses_duplicate() {
    let (_dir, mut e) = project();
    let albedo = AssetId::new(AssetType::Texture, "textures/rock_albedo");
    let normal = AssetId::new(AssetType::Texture, "textures/rock_normal");
    let set_path = |p: &str| Action::Texture(TextureAction::SetPath(p.into()));
    e.apply(Action::OpenAsset(albedo.clone()));

    // The file of textures/rock_normal.
    e.apply(set_path("textures/rock_normal.png"));
    assert_eq!(texture_doc(&e).asset.path, "textures/rock_albedo.ktx2", "model unchanged");
    assert!(!e.is_dirty());
    let last = e.messages.last().unwrap();
    assert_eq!(last.level, Level::Error);
    assert!(
        last.text.contains("textures/rock_normal.png") && last.text.contains(&normal.source_rel()),
        "{}",
        last.text
    );
    assert!(matches!(e.dialog, Some(Dialog::Message { .. })));
    e.apply(Action::CloseDialog);

    // An invalid path (E012 rules) is refused too.
    e.apply(set_path("../outside.png"));
    assert_eq!(texture_doc(&e).asset.path, "textures/rock_albedo.ktx2");
    assert!(e.messages.last().unwrap().text.contains("must not contain"), "{}", e.messages.last().unwrap().text);
    e.apply(Action::CloseDialog);

    // A free file is taken (a missing one is only W109).
    e.apply(set_path(" textures/rock_albedo.png "));
    assert_eq!(texture_doc(&e).asset.path, "textures/rock_albedo.png");
    assert!(e.is_dirty());

    // Claims are disk OR open tabs: the file rock_albedo left is still its own on disk until it is saved, and the new
    // one is claimed by the unsaved tab. Neither can be taken by a path edit or "Create Texture Asset".
    let ktx2 = "textures/rock_albedo.ktx2";
    let before = e.disk.textures.len();
    assert_eq!(e.texture_for_file(ktx2), Some(albedo.clone()));
    assert_eq!(e.create_texture_for_file(ktx2), None);
    assert!(e.messages.last().unwrap().text.contains(&albedo.source_rel()));
    e.apply(Action::CloseDialog);
    assert_eq!(e.texture_for_file("textures/rock_albedo.png"), Some(albedo.clone()));
    assert_eq!(e.create_texture_for_file("textures/rock_albedo.png"), None);
    e.apply(Action::CloseDialog);
    assert_eq!(e.disk.textures.len(), before, "nothing written");
    e.apply(Action::OpenAsset(normal.clone()));
    for taken in [ktx2, "textures/rock_albedo.png"] {
        e.apply(set_path(taken));
        assert_eq!(texture_doc(&e).asset.path, "textures/rock_normal.png", "{taken}");
        assert!(e.messages.last().unwrap().text.contains(&albedo.source_rel()), "{taken}");
        e.apply(Action::CloseDialog);
    }

    // Saved: the old file is free.
    e.apply(Action::OpenAsset(albedo.clone()));
    e.apply(Action::Save);
    e.apply(Action::OpenAsset(normal.clone()));
    e.apply(set_path(ktx2));
    assert_eq!(texture_doc(&e).asset.path, ktx2);

    // Undo in rock_albedo would point it back at that file, now claimed by the unsaved rock_normal tab: refused.
    e.apply(Action::OpenAsset(albedo.clone()));
    e.apply(Action::Undo);
    assert_eq!(texture_doc(&e).asset.path, "textures/rock_albedo.png");
    assert!(e.messages.last().unwrap().text.contains(&normal.source_rel()));
    e.apply(Action::CloseDialog);

    // Saving rock_normal and regenerating from disk: no two assets on one file.
    e.apply(Action::OpenAsset(normal.clone()));
    e.apply(Action::Save);
    e.validate();
    assert!(!e.all_diagnostics().iter().any(|d| d.code == Code::E011), "{:?}", e.all_diagnostics());
}

/// Internal textures (design §2) open in a read-only Texture tab: double click in the `Internal` folder, the
/// "Open texture" button of a node using one, the config.
#[test]
fn internal_texture_tab_is_read_only() {
    use crate::tab::TabKey;
    use crate::ui::tree::{self, TreeItem, TreeRow};
    use shader_graph_core::texture::TextureSettings;
    use shader_stdlib::internal::{DEFAULT_TEXTURE, INTERNAL_TEXTURES};
    let (dir, mut e) = project();
    let id = AssetId::new(AssetType::Texture, DEFAULT_TEXTURE);
    let row = TreeRow { tree: TreeId::Project, item: TreeItem::Internal(DEFAULT_TEXTURE.into()) };
    assert!(tree::click_actions(&row, 1).is_empty());
    let open = tree::click_actions(&row, 2);
    assert_eq!(open, vec![Action::OpenAsset(id.clone())]);
    for a in open.clone() {
        e.apply(a);
    }
    assert_eq!(e.tabs.len(), 1);
    let doc = e.tab().unwrap().doc();
    assert_eq!(doc.view(), ViewKind::Texture);
    assert!(doc.is_read_only());
    assert_eq!(doc.asset(), None, "not a project asset");
    assert_eq!(e.tab().unwrap().owner(), None);
    assert_eq!(doc.base_title(), DEFAULT_TEXTURE);
    let key = doc.config_key().unwrap();
    assert_eq!(key, DEFAULT_TEXTURE);
    assert_eq!(parse_config_key(&key), Some(TabKey::Internal(DEFAULT_TEXTURE.into())));
    let t = texture_doc(&e);
    assert_eq!(t.asset.settings, (INTERNAL_TEXTURES[0].settings)());
    assert_eq!(t.asset.path, "shaders/internal/default.png", "where regeneration copies it");
    assert_eq!(t.image_key(), DEFAULT_TEXTURE, "the preview decodes the embedded bytes");
    assert!(!e.overridden_set().textures.contains_key(&id));

    // Edits, undo and save do nothing.
    let settings = t.asset.settings.clone();
    e.apply(Action::Texture(TextureAction::SetSettings(TextureSettings::default())));
    e.apply(Action::Texture(TextureAction::SetPath("textures/x.png".into())));
    e.apply(Action::Undo);
    assert_eq!(texture_doc(&e).asset.settings, settings);
    assert_eq!(texture_doc(&e).asset.path, "shaders/internal/default.png");
    assert!(!e.is_dirty());
    assert!(!e.save(0));
    assert!(!dir.path().join("shaders/@internal").exists());

    // Opened again from a node referencing it: the same tab.
    let rock = AssetId::new(AssetType::Material, "env/rock_wet");
    e.apply(Action::OpenAsset(rock));
    let sample = add(&mut e, "stdlib::TextureSample");
    e.apply(Action::SetValue {
        target: PropTarget::Param { node: sample.clone(), key: "texture".into() },
        value: Value::Str(DEFAULT_TEXTURE.into()),
        merge: false,
    });
    assert_eq!(e.referenced_asset(&sample), Some(id.clone()));
    assert!(e.texture_choices().iter().any(|(v, _)| v == DEFAULT_TEXTURE), "the picker lists internal textures");
    assert!(e.texture_choices().iter().any(|(v, _)| v == "textures/rock_albedo"));
    e.apply(Action::OpenAsset(id.clone()));
    assert_eq!(e.tabs.len(), 2);
    assert_eq!(e.active, Some(0));

    // Restored on the next launch.
    e.save_tabs_to_config();
    let mut next = Editor::with_config(e.config.clone(), None);
    next.open_project(dir.path()).unwrap();
    assert!(next.tabs.iter().any(|t| matches!(t, Tab::Texture(t) if t.id == id)));
}

// ---------------------------------------------------------------------------------------------
// Drag and drop onto graphs, Convert to Parameter (design §4.3, §4.6)

fn game_file(dir: &tempfile::TempDir, e: &mut Editor, rel: &str) {
    let p = dir.path().join("assets").join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, b"x").unwrap();
    e.refresh_game_files();
}

fn drop_file(e: &mut Editor, path: &str, pos: (f32, f32), node: Option<&str>) {
    let item = crate::droppable::Dropped::GameFile(path.into());
    e.apply(Action::Drop { item, pos, node: node.map(String::from) });
}

fn nodes_of(e: &Editor, kind: &str) -> Vec<NodeId> {
    e.doc().unwrap().asset.nodes.iter().filter(|(_, n)| n.kind == kind).map(|(k, _)| k.clone()).collect()
}

fn param_of(e: &Editor, node: &str, key: &str) -> Option<Value> {
    e.doc().unwrap().asset.nodes[node].params.get(key).cloned()
}

fn open_rock_wet(e: &mut Editor) {
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet")));
}

#[test]
fn drop_image_creates_texture_asset_and_node() {
    let (dir, mut e) = project();
    game_file(&dir, &mut e, "textures/env/Rock-Albedo.png");
    open_rock_wet(&mut e);
    assert!(nodes_of(&e, "stdlib::TextureSample").is_empty());
    drop_file(&mut e, "textures/env/Rock-Albedo.png", (12.0, 34.0), None);

    let id = AssetId::new(AssetType::Texture, "textures/env/rock_albedo");
    assert!(dir.path().join("shaders/textures/env/rock_albedo.texture.ron").is_file());
    assert_eq!(e.disk.textures[&id].as_ref().unwrap().path, "textures/env/Rock-Albedo.png");
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
    use crate::droppable::Dropped;
    use crate::ui::tree::{TreeItem, TreeRow};
    let (dir, mut e) = project();
    // The fixture `textures/rock_albedo` points at this file.
    game_file(&dir, &mut e, "textures/rock_albedo.ktx2");
    open_rock_wet(&mut e);
    let count = e.disk.textures.len();
    drop_file(&mut e, "textures/rock_albedo.ktx2", (0.0, 0.0), None);
    assert_eq!(e.disk.textures.len(), count, "no new Texture asset");
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
        Dropped::from_row(&row(TreeId::Project, TreeItem::Asset(tex))),
        Some(Dropped::Texture("textures/atlas".into()))
    );
    assert_eq!(
        Dropped::from_row(&row(TreeId::Project, TreeItem::Internal("@internal/default".into()))),
        Some(Dropped::Texture("@internal/default".into()))
    );
    assert_eq!(Dropped::from_row(&row(TreeId::Project, TreeItem::Asset(mat))), None);
    assert_eq!(Dropped::from_row(&row(TreeId::GameAssets, TreeItem::GameFile("a/b.txt".into()))), None);
    assert_eq!(Dropped::from_row(&row(TreeId::GameAssets, TreeItem::Folder("a".into()))), None);

    // A Texture asset from the Project tree: the same node, nothing created.
    e.apply(Action::Drop { item: Dropped::Texture("textures/atlas".into()), pos: (5.0, 5.0), node: None });
    assert_eq!(e.disk.textures.len(), count);
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
    e.apply(Action::Undo);
    assert_eq!(param_of(&e, &albedo, "default"), Some(Value::Str("textures/rock_albedo".into())));

    // A plain TextureSample: its `texture` param; a built-in texture works the same way.
    let sample = add(&mut e, "stdlib::TextureSample");
    let item = crate::droppable::Dropped::Texture("@internal/default".into());
    e.apply(Action::Drop { item, pos: (0.0, 0.0), node: Some(sample.clone()) });
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
    use crate::droppable::Dropped;
    use crate::ui::tree::{TreeItem, TreeRow};
    let (_dir, mut e) = project();
    let row = |id| TreeRow { tree: TreeId::Project, item: TreeItem::Asset(id) };
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
    e.apply(Action::Drop { item, pos: (12.0, 34.0), node: Some(out) });
    let added: Vec<NodeId> =
        nodes_of(&e, kind).into_iter().filter(|n| e.doc().unwrap().selection.contains(n)).collect();
    assert_eq!(nodes_of(&e, kind).len(), before + 1);
    assert_eq!(added.len(), 1, "the new call node is selected");
    assert_eq!(e.doc().unwrap().asset.nodes[&added[0]].pos, (10.0, 30.0), "snapped to the 10-unit grid");
    e.apply(Action::Undo);
    assert_eq!(nodes_of(&e, kind).len(), before, "one undo step");
}

/// Design §4.11: a function graph cannot call itself, as in the palette.
#[test]
fn drop_function_into_itself_is_rejected() {
    use crate::droppable::Dropped;
    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Function, "common/wet_mask")));
    let before = e.doc().unwrap().asset.clone();
    let warnings = e.messages.iter().filter(|m| m.level == Level::Warning).count();
    let item = Dropped::Function("common/wet_mask".into());
    e.apply(Action::Drop { item, pos: (0.0, 0.0), node: None });
    assert_eq!(e.doc().unwrap().asset, before, "nothing added");
    assert!(!e.is_dirty());
    let new: Vec<&Message> = e.messages.iter().filter(|m| m.level == Level::Warning).skip(warnings).collect();
    assert!(new.len() == 1 && new[0].text.contains("common/wet_mask"), "{new:?}");

    // Another function is fine.
    let item = Dropped::Function("common/tex_layer".into());
    e.apply(Action::Drop { item, pos: (0.0, 0.0), node: None });
    assert_eq!(nodes_of(&e, "function::common/tex_layer").len(), 1);
}

/// Design §4.11: a Curve asset on empty canvas adds a Sample Curve node reading it.
#[test]
fn drop_curve_adds_sample_curve() {
    use crate::droppable::Dropped;
    use crate::ui::tree::{TreeItem, TreeRow};
    let (_dir, mut e) = project();
    let pulse = AssetId::new(AssetType::Curve, "common/pulse");
    let row = TreeRow { tree: TreeId::Project, item: TreeItem::Asset(pulse.clone()) };
    assert_eq!(Dropped::from_row(&row), Some(Dropped::Curve("common/pulse".into())));

    open_rock_wet(&mut e);
    let before = nodes_of(&e, "stdlib::SampleCurve");
    e.apply(Action::Drop { item: Dropped::Curve("common/pulse".into()), pos: (21.0, 9.0), node: None });
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
    use crate::droppable::Dropped;
    let (_dir, mut e) = project();
    open_rock_wet(&mut e);
    let sample = add(&mut e, "stdlib::SampleCurve");
    e.apply(Action::Drop { item: Dropped::Curve("common/pulse".into()), pos: (0.0, 0.0), node: Some(sample.clone()) });
    assert_eq!(param_of(&e, &sample, "curve"), Some(Value::Str("common/pulse".into())));
    let count = e.doc().unwrap().asset.nodes.len();
    e.apply(Action::Drop { item: Dropped::Curve("common/steps".into()), pos: (0.0, 0.0), node: Some(sample.clone()) });
    assert_eq!(e.doc().unwrap().asset.nodes.len(), count, "no node added");
    assert_eq!(param_of(&e, &sample, "curve"), Some(Value::Str("common/steps".into())));
    assert_eq!(e.doc().unwrap().selection, [sample.clone()].into());
    e.apply(Action::Undo);
    assert_eq!(param_of(&e, &sample, "curve"), Some(Value::Str("common/pulse".into())), "one undo step");
    assert_eq!(e.doc().unwrap().asset.nodes.len(), count);

    // A node without a curve param: a new Sample Curve node instead.
    let out = output_node(&e);
    e.apply(Action::Drop { item: Dropped::Curve("common/steps".into()), pos: (0.0, 0.0), node: Some(out) });
    assert_eq!(e.doc().unwrap().asset.nodes.len(), count + 1);
}

/// Design §4.1: new nodes land on the 10-unit grid; a paste moves its first node onto it and keeps the layout.
#[test]
fn added_and_pasted_nodes_are_snapped() {
    let (_dir, mut e) = project();
    open_rock_wet(&mut e);
    let add = |e: &mut Editor, pos| {
        let before: Vec<_> = e.doc().unwrap().asset.nodes.keys().cloned().collect();
        e.apply(Action::AddNode { kind: "stdlib::Constant".into(), pos, link: None, params: Vec::new() });
        e.doc().unwrap().asset.nodes.keys().find(|k| !before.contains(k)).unwrap().clone()
    };
    let a = add(&mut e, (12.3, 7.6));
    assert_eq!(e.doc().unwrap().asset.nodes[&a].pos, (10.0, 10.0));
    let b = add(&mut e, (0.0, 0.0));
    // Off-grid like a node of an older file (positions are not rewritten on load).
    let moved = vec![(a.clone(), (3.3, 4.4)), (b.clone(), (103.3, 54.4))];
    e.apply(Action::Moved { nodes: moved, frames: Vec::new() });
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
    e.apply(Action::Select([a.clone(), b.clone()].into()));
    e.apply(Action::Copy);
    e.apply(Action::Paste(Some((33.0, 1.0))));
    assert_eq!(check(&e), (30.0, 0.0));
    e.apply(Action::Paste(None));
    check(&e);
    e.apply(Action::Select([a].into()));
    e.apply(Action::Duplicate);
    let d = e.doc().unwrap();
    assert!(on_grid(d.asset.nodes[d.selection.first().unwrap()].pos));

    // A reroute on a wire and a comment frame at the cursor are placed on the grid too.
    let out = output_node(&e);
    let before: Vec<_> = e.doc().unwrap().asset.nodes.keys().cloned().collect();
    e.apply(Action::AddReroute { from: (b, "out".into()), to: (out, "metallic".into()), pos: (-41.0, 22.6) });
    let d = e.doc().unwrap();
    let r = d.asset.nodes.iter().find(|(k, _)| !before.contains(k)).unwrap().1;
    assert_eq!((r.kind.as_str(), r.pos), ("stdlib::Reroute", (-40.0, 20.0)));
    e.apply(Action::Select(BTreeSet::new()));
    e.apply(Action::AddComment(Some((12.3, -7.6))));
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
    e.apply(Action::Undo);
    assert!(nodes_of(&e, "stdlib::TextureSample").is_empty(), "undo removes the node");
    assert!(!e.is_dirty(), "one undo step");
    let id = AssetId::new(AssetType::Texture, "textures/env/rock_albedo");
    assert!(dir.path().join("shaders/textures/env/rock_albedo.texture.ron").is_file(), "the asset stays");
    assert!(e.disk.textures.contains_key(&id));
    // Dropped again: reused, not `rock_albedo_2`.
    drop_file(&mut e, "textures/env/Rock-Albedo.png", (0.0, 0.0), None);
    let added = nodes_of(&e, "stdlib::TextureSample");
    assert_eq!(param_of(&e, &added[0], "texture"), Some(Value::Str(id.path.clone())));
    assert!(!dir.path().join("shaders/textures/env/rock_albedo_2.texture.ron").exists());
}

/// Applies the node menu row labelled `label` of `node`.
fn menu_action(e: &mut Editor, node: &str, label: &str) {
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
    let set = |key: &str, v: &str| Action::SetValue {
        target: PropTarget::Param { node: sample.clone(), key: key.into() },
        value: Value::Str(v.into()),
        merge: false,
    };
    e.apply(set("texture", "textures/rock_albedo"));
    e.apply(set("mip_mode", "Bias"));
    e.apply(set("sampler_type", "Normal"));
    let out = output_node(&e);
    e.apply(Action::Disconnect { to: (out.clone(), "roughness".into()) });
    e.apply(Action::Connect { from: (sample.clone(), "r".into()), to: (out.clone(), "roughness".into()) });
    e.apply(Action::SetValue {
        target: PropTarget::Input { node: sample.clone(), pin: "mip".into() },
        value: Value::Float(2.0),
        merge: false,
    });

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
    e.apply(Action::SetValue {
        target: PropTarget::Param { node: obj.clone(), key: "texture".into() },
        value: Value::Str("textures/atlas".into()),
        merge: false,
    });
    e.apply(Action::Connect { from: (obj.clone(), "out".into()), to: (sample.clone(), "tex".into()) });
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
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Function, "common/tex_layer")));
    let in_fn = node_of(&e, "stdlib::TextureSample");
    assert!(!e.node_menu(&in_fn).iter().any(|(l, _)| l.starts_with("Convert")), "{:?}", e.node_menu(&in_fn));
}

#[test]
fn convert_two_nodes_get_distinct_unique_names() {
    let (_dir, mut e) = project();
    open_rock_wet(&mut e);
    let a = add(&mut e, "stdlib::TextureSample");
    let b = add(&mut e, "stdlib::TextureSample");
    e.apply(Action::Select([a.clone(), b.clone()].into()));
    menu_action(&mut e, &a, "Convert to Parameter");
    let name = |n: &str| param_of(&e, n, "name").map(|v| v.as_str().to_string()).unwrap_or_default();
    assert_eq!(nodes_of(&e, "param::TextureSample2D").len(), 4, "both converted (rock_wet has two)");
    let names: BTreeSet<String> = [name(&a), name(&b)].into();
    assert_eq!(names, ["texture".to_string(), "texture_1".to_string()].into());
    e.validate();
    assert!(!e.all_diagnostics().iter().any(Diagnostic::is_error), "{:?}", e.all_diagnostics());
}

#[test]
fn quick_start_switches_steps() {
    let (_dir, mut e) = project();
    e.apply(Action::QuickStart(0));
    assert_eq!(e.dialog, Some(Dialog::QuickStart(0)));
    e.apply(Action::QuickStart(5));
    assert_eq!(e.dialog, Some(Dialog::QuickStart(5)), "a step tab switches the open dialog");
    e.apply(Action::CloseDialog);
    assert_eq!(e.dialog, None);
}

/// Texture tab Easy mode (decision 0028): presets are undoable settings edits, the mode and the channel view are
/// editor-only.
#[test]
fn texture_tab_presets_mode_and_channels() {
    use crate::texture_doc::FilterPreset;
    use shader_graph_core::texture::{AddressMode, TextureSettings};
    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Texture, "textures/rock_albedo")));
    assert!(texture_doc(&e).easy, "a default sampler opens in Easy mode");
    e.apply(Action::Texture(TextureAction::Preset(FilterPreset::Ground)));
    let s = texture_doc(&e).asset.settings.sampler.unwrap();
    assert_eq!(Some(s), FilterPreset::Ground.sampler());
    assert_eq!(s.address_mode_u, AddressMode::Repeat);
    e.apply(Action::Texture(TextureAction::Preset(FilterPreset::Ground)));
    e.apply(Action::Texture(TextureAction::Preset(FilterPreset::PixelArt)));
    e.apply(Action::Undo);
    assert_eq!(FilterPreset::of(&texture_doc(&e).asset.settings), Some(FilterPreset::Ground), "one step each");
    e.apply(Action::Undo);
    assert_eq!(texture_doc(&e).asset.settings, TextureSettings::default());
    assert!(!e.is_dirty());

    let layout = texture_doc(&e).layout;
    e.apply(Action::Texture(TextureAction::SetEasy(false)));
    assert!(!texture_doc(&e).easy && texture_doc(&e).layout != layout, "the form is rebuilt");
    assert!(!e.is_dirty(), "the mode is not part of the asset");

    e.apply(Action::PreviewChannel(Some(1)));
    assert_eq!(texture_doc(&e).channel, Some(1));
    e.apply(Action::PreviewChannel(Some(7)));
    assert_eq!(texture_doc(&e).channel, None, "out of range: the whole image");
    e.apply(Action::SetValue {
        target: PropTarget::PreviewTiled,
        value: shader_graph_core::Value::Bool(true),
        merge: false,
    });
    assert!(texture_doc(&e).tiled);
    assert!(!e.is_dirty());
}

/// Design §6.3: the Easy preset is stored only through the sampler it writes, so every preset comes back selected
/// after save and reopen; "Default" writes no `sampler` key at all (the untouched file, byte for byte).
#[test]
fn easy_preset_round_trips_through_file() {
    use crate::texture_doc::FilterPreset;
    let (dir, mut e) = project();
    let id = AssetId::new(AssetType::Texture, "textures/sprite");
    let file = dir.path().join("shaders/textures/sprite.texture.ron");
    let untouched = std::fs::read(&file).unwrap();
    assert!(!String::from_utf8_lossy(&untouched).contains("sampler"));
    e.apply(Action::OpenAsset(id.clone()));
    assert!(texture_doc(&e).easy, "no sampler opens in Easy");
    assert_eq!(FilterPreset::of(&texture_doc(&e).asset.settings), Some(FilterPreset::Default), "Default selected");
    // Default last: back to the untouched file.
    for p in FilterPreset::ALL.into_iter().skip(1).chain([FilterPreset::Default]) {
        e.apply(Action::Texture(TextureAction::Preset(p)));
        e.apply(Action::Save);
        assert!(!e.is_dirty(), "{p:?} saved");
        e.apply(Action::CloseTab(e.active.unwrap()));
        assert!(e.tabs.is_empty());
        e.apply(Action::OpenAsset(id.clone()));
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
    use crate::texture_doc::{FilterPreset, TextureField};
    use shader_graph_core::Value;
    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Texture, "textures/sprite")));
    e.apply(Action::Texture(TextureAction::SetEasy(false)));
    let custom =
        TextureField::LodMaxClamp.set(&FilterPreset::ThreeD.apply(&texture_doc(&e).asset.settings), &Value::Float(4.0));
    e.apply(Action::Texture(TextureAction::SetSettings(custom.clone())));
    assert_eq!(FilterPreset::of(&custom), None);
    let layout = texture_doc(&e).layout;
    e.apply(Action::Texture(TextureAction::SetEasy(true)));
    assert!(!texture_doc(&e).easy, "refused: Easy would hide the custom sampler");
    assert_eq!(texture_doc(&e).asset.settings, custom, "settings untouched");
    assert_eq!(texture_doc(&e).layout, layout, "nothing to rebuild");
    // Back on a preset, Easy is allowed again.
    e.apply(Action::Texture(TextureAction::Preset(FilterPreset::ThreeD)));
    e.apply(Action::Texture(TextureAction::SetEasy(true)));
    assert!(texture_doc(&e).easy);
}

/// Design §6.3: undo, redo and an external reload that bring back a sampler matching no preset leave Easy (the form
/// shows the custom values); ones that keep a preset keep the mode.
#[test]
fn history_and_reload_leave_easy_for_custom_sampler() {
    use crate::texture_doc::{FilterPreset, TextureField};
    use shader_graph_core::Value;
    let (dir, mut e) = project();
    let id = AssetId::new(AssetType::Texture, "textures/sprite");
    e.apply(Action::OpenAsset(id.clone()));
    e.apply(Action::Texture(TextureAction::SetEasy(false)));
    let lod = Value::Float(4.0);
    let custom = TextureField::LodMaxClamp.set(&texture_doc(&e).asset.settings, &lod);
    e.apply(Action::Texture(TextureAction::SetSettings(custom.clone())));
    let default = Value::Bool(true);
    e.apply(Action::SetValue {
        target: PropTarget::Texture(TextureField::DefaultSampler),
        value: default,
        merge: false,
    });
    e.apply(Action::Texture(TextureAction::SetEasy(true)));
    assert!(texture_doc(&e).easy, "Default fits Easy");

    // Undo brings the custom sampler back: Advanced, with the form rebuilt.
    let layout = texture_doc(&e).layout;
    e.apply(Action::Undo);
    assert_eq!(texture_doc(&e).asset.settings, custom);
    assert!(!texture_doc(&e).easy, "undo to a custom sampler leaves Easy");
    assert_ne!(texture_doc(&e).layout, layout);

    // Redo to a preset keeps Advanced (the mode is the user's); a redo to the custom sampler leaves Easy.
    e.apply(Action::Redo);
    assert_eq!(FilterPreset::of(&texture_doc(&e).asset.settings), Some(FilterPreset::Default));
    assert!(!texture_doc(&e).easy);
    e.apply(Action::Undo);
    e.apply(Action::Undo);
    assert_eq!(FilterPreset::of(&texture_doc(&e).asset.settings), Some(FilterPreset::Default), "the untouched asset");
    e.apply(Action::Texture(TextureAction::SetEasy(true)));
    e.apply(Action::Redo);
    assert_eq!(texture_doc(&e).asset.settings, custom);
    assert!(!texture_doc(&e).easy, "redo to a custom sampler leaves Easy");

    // An undo to a preset in Easy stays in Easy.
    e.apply(Action::Redo);
    e.apply(Action::Texture(TextureAction::SetEasy(true)));
    e.apply(Action::Texture(TextureAction::Preset(FilterPreset::Ground)));
    e.apply(Action::Undo);
    assert!(texture_doc(&e).easy, "undo to Default keeps Easy");

    // An external edit of the (saved) file to a custom sampler: reloaded, out of Easy.
    e.apply(Action::Save);
    assert!(texture_doc(&e).easy && !e.is_dirty());
    let file = dir.path().join("shaders/textures/sprite.texture.ron");
    let text = "TextureAsset(\n  format_version: 1,\n  path: \"textures/sprite.png\",\n  settings: TextureSettings(\n    \
                sampler: Some(SamplerSettings(\n      lod_max_clamp: 4.0,\n    )),\n  ),\n)\n";
    std::fs::write(&file, text).unwrap();
    e.apply(Action::Reload(id));
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
    let image = |e: &Editor| e.tab().and_then(Tab::image).map(|i| (i.channel, i.tiled)).unwrap();
    assert_eq!(image(&e), (None, false));
    e.apply(Action::PreviewChannel(Some(2)));
    e.apply(Action::SetValue {
        target: PropTarget::PreviewTiled,
        value: shader_graph_core::Value::Bool(true),
        merge: false,
    });
    assert_eq!(image(&e), (Some(2), true));
    assert_eq!(e.current_value(&PropTarget::PreviewTiled), Some(shader_graph_core::Value::Bool(true)));
    assert!(!e.tab().unwrap().doc().is_dirty());
    e.apply(Action::Undo);
    assert_eq!(image(&e), (Some(2), true), "not undoable");
}

/// A click on the editor statusbar's problem count focuses what the first console/logs row of this asset focuses.
#[test]
fn statusbar_problem_is_first_diagnostic_of_active_asset() {
    let (_dir, mut e) = project();
    assert!(e.active_first_diagnostic().is_none(), "no tab");
    let rock = AssetId::new(AssetType::Material, "env/rock_wet");
    e.apply(Action::OpenAsset(rock.clone()));
    e.apply(Action::AddNode { kind: "stdlib::Time".into(), pos: (0.0, 0.0), link: None, params: Vec::new() });
    e.validate();
    let first = e.diags.get(&rock).and_then(|d| d.first()).expect("rock_wet has warnings");
    assert_eq!(e.active_first_diagnostic(), Some(first));
    e.apply(Action::OpenAsset(pulse()));
    assert!(e.active_first_diagnostic().is_none(), "the curve has no problems");
}

// Ctrl+LMB drag moves wires (decision 0032).

fn edges(e: &Editor) -> Vec<(NodeId, String, NodeId, String)> {
    let d = e.doc().unwrap();
    let mut v: Vec<_> =
        d.asset.edges.iter().map(|x| (x.from.0.clone(), x.from.1.clone(), x.to.0.clone(), x.to.1.clone())).collect();
    v.sort();
    v
}

fn connect(e: &mut Editor, from: &NodeId, from_pin: &str, to: &NodeId, to_pin: &str) {
    e.apply(Action::Connect { from: (from.clone(), from_pin.into()), to: (to.clone(), to_pin.into()) });
    let source = e.doc().unwrap().asset.edge_into(to, to_pin).map(|x| x.from.clone());
    assert_eq!(source, Some((from.clone(), from_pin.into())));
}

fn move_wires(node: &NodeId, pin: &str, output: bool, to: (&NodeId, &str)) -> Action {
    Action::MoveWires { node: node.clone(), pin: pin.into(), output, to: (to.0.clone(), to.1.into()) }
}

#[test]
fn move_input_wire_to_another_input() {
    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet")));
    let (a, b, c) = (add(&mut e, "stdlib::Constant"), add(&mut e, "stdlib::Lerp"), add(&mut e, "stdlib::Lerp"));
    connect(&mut e, &a, "out", &b, "a");
    let before = edges(&e);
    e.apply(move_wires(&b, "a", false, (&c, "b")));
    let d = e.doc().unwrap();
    assert!(d.asset.edge_into(&b, "a").is_none());
    let moved = d.asset.edge_into(&c, "b").expect("moved");
    assert_eq!(moved.from, (a.clone(), "out".into()));
    assert_eq!(edges(&e).len(), before.len());
    e.apply(Action::Undo);
    assert_eq!(edges(&e), before, "one undo step");
}

#[test]
fn move_output_wires_to_another_output() {
    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet")));
    let (a, d) = (add(&mut e, "stdlib::Constant"), add(&mut e, "stdlib::Constant"));
    let (b, c) = (add(&mut e, "stdlib::Lerp"), add(&mut e, "stdlib::Lerp"));
    connect(&mut e, &a, "out", &b, "a");
    connect(&mut e, &a, "out", &c, "b");
    let before = edges(&e);
    e.apply(move_wires(&a, "out", true, (&d, "out")));
    let asset = &e.doc().unwrap().asset;
    assert!(!asset.edges.iter().any(|x| x.from.0 == a), "{:?}", asset.edges);
    assert_eq!(asset.edge_into(&b, "a").unwrap().from, (d.clone(), "out".into()));
    assert_eq!(asset.edge_into(&c, "b").unwrap().from, (d.clone(), "out".into()));
    e.apply(Action::Undo);
    assert_eq!(edges(&e), before, "one undo step");
}

#[test]
fn move_wires_rejects_incompatible() {
    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet")));
    let out = output_node(&e);
    let (v3, lerp) = (add(&mut e, "stdlib::Constant3Vector"), add(&mut e, "stdlib::Lerp"));
    let make = add(&mut e, "stdlib::MakeMaterialAttributesPbr");
    connect(&mut e, &v3, "out", &lerp, "a");
    connect(&mut e, &v3, "out", &out, "base_color");
    let (before, rev) = (edges(&e), e.doc().unwrap().revision);
    // The Lerp takes material attributes, `base_color` does not: nothing moves.
    e.apply(move_wires(&v3, "out", true, (&make, "attributes")));
    assert_eq!(edges(&e), before);
    assert_eq!(e.doc().unwrap().revision, rev, "no edit recorded");
    let last = e.messages.last().expect("a message");
    assert_eq!(last.level, Level::Warning);
    assert!(last.text.contains("base_color"), "names the failing wire: {}", last.text);
}

#[test]
fn move_wires_rejects_direction_mismatch() {
    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet")));
    let (a, b, c) = (add(&mut e, "stdlib::Constant"), add(&mut e, "stdlib::Lerp"), add(&mut e, "stdlib::Constant"));
    connect(&mut e, &a, "out", &b, "a");
    let (before, rev) = (edges(&e), e.doc().unwrap().revision);
    e.apply(move_wires(&b, "a", false, (&c, "out")));
    assert_eq!(edges(&e), before, "input wire onto an output");
    e.apply(move_wires(&a, "out", true, (&b, "b")));
    assert_eq!(edges(&e), before, "output wires onto an input");
    e.apply(move_wires(&b, "a", false, (&b, "a")));
    assert_eq!(edges(&e), before, "onto itself");
    assert_eq!(e.doc().unwrap().revision, rev);
}

/// A move that would close a loop changes nothing, also when only a later wire of an output does (Review Focus 4).
#[test]
fn move_wires_rejects_cycle() {
    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet")));
    let reject = |e: &mut Editor, action: Action, what: &str| {
        let (before, rev, n) = (edges(e), e.doc().unwrap().revision, e.messages.len());
        e.apply(action);
        assert_eq!(edges(e), before, "{what}");
        assert_eq!(e.doc().unwrap().revision, rev, "{what}: no edit recorded");
        assert_eq!(e.messages.len(), n + 1, "{what}: one message");
        let last = e.messages.last().unwrap();
        assert_eq!(last.level, Level::Warning, "{what}");
        assert!(last.text.contains("E002"), "{what}: {}", last.text);
    };

    // Input: l1 → l2 → l3.a; moving l3.a's wire (from l2) onto l1.b closes l1 → l2 → l1.
    let (c, l1, l2, l3) = (
        add(&mut e, "stdlib::Constant"),
        add(&mut e, "stdlib::Lerp"),
        add(&mut e, "stdlib::Lerp"),
        add(&mut e, "stdlib::Lerp"),
    );
    connect(&mut e, &c, "out", &l1, "a");
    connect(&mut e, &l1, "out", &l2, "a");
    connect(&mut e, &l2, "out", &l3, "a");
    reject(&mut e, move_wires(&l3, "a", false, (&l1, "b")), "input move");

    // Output: k.out feeds two Lerps; the one whose wire is checked second (edge order) feeds z. Moving k's wires onto
    // z.out: the first rewired wire is fine, the second closes second → z → second.
    let (k, p, q, z) = (
        add(&mut e, "stdlib::Constant"),
        add(&mut e, "stdlib::Lerp"),
        add(&mut e, "stdlib::Lerp"),
        add(&mut e, "stdlib::Lerp"),
    );
    connect(&mut e, &k, "out", &p, "a");
    connect(&mut e, &k, "out", &q, "a");
    let order: Vec<_> = e.doc().unwrap().asset.edges.iter().filter(|w| w.from.0 == k).map(|w| w.to.0.clone()).collect();
    assert_eq!(order.len(), 2);
    connect(&mut e, &order[1], "out", &z, "a");
    reject(&mut e, move_wires(&k, "out", true, (&z, "out")), "output move");
}

/// The wire an input move replaces is not part of the check: `Add.a` fed a Float4 takes a Float moved onto it.
#[test]
fn move_wires_validates_without_replaced_wire() {
    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet")));
    let (v4, sum) = (add(&mut e, "stdlib::Constant4Vector"), add(&mut e, "stdlib::Add"));
    let (f, lerp) = (add(&mut e, "stdlib::Constant"), add(&mut e, "stdlib::Lerp"));
    connect(&mut e, &v4, "out", &sum, "a");
    connect(&mut e, &f, "out", &lerp, "b");
    let before = edges(&e);
    let from = (lerp.clone(), "b".to_string());
    let to = (sum.clone(), "a".to_string());
    let planned = e.plan_move_wires(&from, false, &to).expect("the replaced Float4 wire is ignored");
    assert_eq!(planned.edge_into(&sum, "a").unwrap().from, (f.clone(), "out".into()));
    assert!(planned.edge_into(&lerp, "b").is_none());
    assert!(!planned.edges.iter().any(|x| x.from.0 == v4), "the Float4 wire is replaced");
    e.apply(move_wires(&lerp, "b", false, (&sum, "a")));
    assert_eq!(e.doc().unwrap().asset.edges, planned.edges, "applies the plan");
    e.apply(Action::Undo);
    assert_eq!(edges(&e), before, "one undo step");
}

/// The dry run a Ctrl+drag repeats while it hovers pins (Task 10) changes nothing and reports nothing.
#[test]
fn plan_move_wires_is_pure() {
    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet")));
    let (a, b, c) = (add(&mut e, "stdlib::Constant"), add(&mut e, "stdlib::Lerp"), add(&mut e, "stdlib::Lerp"));
    connect(&mut e, &a, "out", &b, "a");
    let (before, rev, messages, props) = (edges(&e), e.doc().unwrap().revision, e.messages.len(), e.ui.props);
    let ok = e.plan_move_wires(&(b.clone(), "a".into()), false, &(c.clone(), "b".into()));
    assert!(ok.is_ok_and(|x| x.edge_into(&c, "b").is_some()));
    let refused = e.plan_move_wires(&(b.clone(), "a".into()), false, &(a.clone(), "out".into()));
    assert!(refused.is_err_and(|why| why.contains("is not an input")));
    assert_eq!(edges(&e), before);
    assert_eq!(e.doc().unwrap().revision, rev, "no revision bump");
    assert_eq!((e.messages.len(), e.ui.props), (messages, props), "no message, no redraw");
}

/// The statusbar's WESL line counts follow the asset: moved on rename, dropped on delete.
#[test]
fn wesl_lines_follow_delete_and_rename() {
    let (_dir, mut e) = project();
    e.regenerate();
    let id = AssetId::new(AssetType::Material, "env/rock_wet");
    assert!(e.wesl_lines.contains_key(&id));
    e.apply(Action::MoveTo(id.clone(), "env2".into()));
    let moved = AssetId::new(AssetType::Material, "env2/rock_wet");
    assert!(!e.wesl_lines.contains_key(&id), "the old id has no entry");
    assert!(e.wesl_lines.contains_key(&moved), "the new id has one");
    e.apply(Action::Delete(moved.clone()));
    assert!(!e.wesl_lines.contains_key(&moved), "dropped on delete");
}

/// The editor statusbar keys on `ui.tabs` instead of the tab's config key: another tab at the same index, or the
/// same tab renamed, must bump it.
#[test]
fn tabs_revision_follows_tab_identity() {
    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet")));
    let (active, rev) = (e.active, e.ui.tabs);
    e.apply(Action::CloseTab(0));
    e.apply(Action::OpenAsset(pulse()));
    assert_eq!(e.active, active, "same index");
    assert_ne!(e.ui.tabs, rev, "another tab");
    let rev = e.ui.tabs;
    e.apply(Action::MoveTo(pulse(), "moved".into()));
    assert_ne!(e.ui.tabs, rev, "renamed");
}

#[test]
fn connect_replaces_existing_input_edge() {
    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet")));
    let (a, b, c) = (add(&mut e, "stdlib::Constant"), add(&mut e, "stdlib::Lerp"), add(&mut e, "stdlib::Constant"));
    connect(&mut e, &a, "out", &b, "a");
    connect(&mut e, &c, "out", &b, "a");
    let into: Vec<_> = e.doc().unwrap().asset.edges.iter().filter(|x| x.to == (b.clone(), "a".into())).collect();
    assert_eq!(into.len(), 1);
    assert_eq!(into[0].from, (c.clone(), "out".into()));
}

/// Three new Constant nodes in rock_wet and a comment frame around the first `framed` of them; returns the node
/// ids and the frame id.
fn framed_nodes(e: &mut Editor, framed: &[usize]) -> (Vec<NodeId>, String) {
    open_rock_wet(e);
    let mut ids = Vec::new();
    for i in 0..3 {
        let before: Vec<_> = e.doc().unwrap().asset.nodes.keys().cloned().collect();
        let pos = (-900.0 + 200.0 * i as f32, 600.0);
        e.apply(Action::AddNode { kind: "stdlib::Constant".into(), pos, link: None, params: Vec::new() });
        ids.push(e.doc().unwrap().asset.nodes.keys().find(|k| !before.contains(k)).unwrap().clone());
    }
    let frames: Vec<_> = e.doc().unwrap().asset.comments.iter().map(|f| f.id.clone()).collect();
    e.apply(Action::Select(framed.iter().map(|i| ids[*i].clone()).collect()));
    e.apply(Action::AddComment(None));
    let f = e.doc().unwrap().asset.comments.iter().find(|f| !frames.contains(&f.id)).unwrap().id.clone();
    (ids, f)
}

fn has_frame(e: &Editor, f: &str) -> bool {
    e.doc().unwrap().asset.comments.iter().any(|c| c.id == f)
}

#[test]
fn deleting_nodes_with_enclosing_frame_removes_it() {
    let (_dir, mut e) = project();
    let (ids, f) = framed_nodes(&mut e, &[0, 1]);
    let (a, b) = (ids[0].clone(), ids[1].clone());
    e.apply(Action::Select([a.clone(), b.clone()].into()));
    e.apply(Action::SelectFrames([f.clone()].into()));
    assert_eq!(e.doc().unwrap().enclosed_frames, BTreeSet::from([f.clone()]));
    e.apply(Action::DeleteSelection);
    let d = e.doc().unwrap();
    assert!(!d.asset.nodes.contains_key(&a) && !d.asset.nodes.contains_key(&b));
    assert!(!has_frame(&e, &f));
    assert!(e.doc().unwrap().enclosed_frames.is_empty());
    // One undo step brings back nodes and frame together (Review Focus 4).
    e.apply(Action::Undo);
    let d = e.doc().unwrap();
    assert!(d.asset.nodes.contains_key(&a) && d.asset.nodes.contains_key(&b));
    assert!(has_frame(&e, &f));
    let frame = d.asset.comments.iter().find(|c| c.id == f).unwrap();
    let mut members = vec![a, b];
    members.sort();
    assert_eq!(frame.nodes, members);
}

#[test]
fn frame_kept_when_not_all_nodes_selected() {
    let (_dir, mut e) = project();
    let (ids, f) = framed_nodes(&mut e, &[0, 2]);
    let (a, b, c) = (ids[0].clone(), ids[1].clone(), ids[2].clone());
    e.apply(Action::Select([a.clone(), b.clone()].into()));
    e.apply(Action::SelectFrames([f.clone()].into()));
    assert!(e.doc().unwrap().enclosed_frames.is_empty(), "c of the frame is not selected");
    e.apply(Action::DeleteSelection);
    let d = e.doc().unwrap();
    assert!(!d.asset.nodes.contains_key(&a) && !d.asset.nodes.contains_key(&b));
    assert!(d.asset.nodes.contains_key(&c));
    assert!(has_frame(&e, &f));
}

/// A marked frame whose member survives the delete (output nodes are never deleted) stays (decision 0033).
#[test]
fn frame_kept_when_a_member_survives_delete() {
    let (_dir, mut e) = project();
    open_rock_wet(&mut e);
    let (out, a) = (output_node(&e), add(&mut e, "stdlib::Constant"));
    let frames: Vec<_> = e.doc().unwrap().asset.comments.iter().map(|f| f.id.clone()).collect();
    e.apply(Action::Select([out.clone(), a.clone()].into()));
    e.apply(Action::AddComment(None));
    let f = e.doc().unwrap().asset.comments.iter().find(|f| !frames.contains(&f.id)).unwrap().id.clone();
    let before = e.doc().unwrap().asset.clone();
    e.apply(Action::Select([out.clone(), a.clone()].into()));
    e.apply(Action::SelectFrames([f.clone()].into()));
    assert_eq!(e.doc().unwrap().enclosed_frames, BTreeSet::from([f.clone()]));
    e.apply(Action::DeleteSelection);
    let d = e.doc().unwrap();
    assert!(!d.asset.nodes.contains_key(&a), "the other node is deleted");
    assert!(d.asset.nodes.contains_key(&out), "the output node is never deleted");
    assert!(has_frame(&e, &f), "its frame stays");
    e.apply(Action::Undo);
    assert_eq!(e.doc().unwrap().asset, before, "one undo step");
}

#[test]
fn select_clears_enclosed_frames() {
    let (_dir, mut e) = project();
    let (ids, f) = framed_nodes(&mut e, &[0, 1]);
    e.apply(Action::Select([ids[0].clone(), ids[1].clone()].into()));
    e.apply(Action::SelectFrames([f.clone()].into()));
    e.apply(Action::Select([ids[0].clone(), ids[1].clone()].into()));
    assert!(e.doc().unwrap().enclosed_frames.is_empty(), "a new node selection drops the mark");
    e.apply(Action::DeleteSelection);
    assert!(has_frame(&e, &f));
    // Without nodes a frame is never marked: clicking a frame selects only the frame (design §4.5).
    e.apply(Action::Select(BTreeSet::new()));
    e.apply(Action::SelectFrames([f.clone()].into()));
    assert!(e.doc().unwrap().enclosed_frames.is_empty());
}

#[test]
fn frame_hue_edit_is_undoable() {
    let (_dir, mut e) = project();
    let (_, f) = framed_nodes(&mut e, &[0]);
    let hue = |e: &Editor| e.doc().unwrap().asset.comments.iter().find(|c| c.id == f).unwrap().hue;
    assert_eq!(hue(&e), None);
    e.apply(Action::SetValue { target: PropTarget::FrameHue(f.clone()), value: Value::Float(200.0), merge: false });
    assert_eq!(hue(&e), Some(200.0));
    e.apply(Action::Undo);
    assert_eq!(hue(&e), None);

    // A Hue drag is one undo step; "Default colour" clears the hue.
    for (v, is_final) in [(10.0, false), (20.0, false), (30.0, true)] {
        e.apply(Action::SetComponent { target: PropTarget::FrameHue(f.clone()), comp: 0, value: v, is_final });
    }
    assert_eq!(hue(&e), Some(30.0));
    e.apply(Action::SetValue {
        target: PropTarget::FrameHue(f.clone()),
        value: Value::Str(String::new()),
        merge: false,
    });
    assert_eq!(hue(&e), None);
    e.apply(Action::Undo);
    assert_eq!(hue(&e), Some(30.0));
    e.apply(Action::Undo);
    assert_eq!(hue(&e), None, "the drag was one step");
}

/// Design §4.10: `min` / `max` set on a Float FunctionInput (unsaved, in the open tab) reach the call node's pin.
#[test]
fn function_input_range_reaches_call_pin() {
    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Function, "common/wet_mask")));
    let input = e
        .doc()
        .unwrap()
        .asset
        .nodes
        .iter()
        .find(|(_, n)| n.kind == "output::FunctionInput" && n.params.get("name") == Some(&Value::Str("wetness".into())))
        .unwrap()
        .0
        .clone();
    for (key, v) in [("min", 0.2), ("max", 5.0)] {
        e.apply(Action::SetValue {
            target: PropTarget::Param { node: input.clone(), key: key.into() },
            value: Value::Float(v),
            merge: false,
        });
    }
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet")));
    // Live validation refreshes the declared function signatures from the open tabs.
    e.validate();
    let call = node_of(&e, "function::common/wet_mask");
    let d = e.doc().unwrap();
    let sig = node_signature(&d.asset, &call, &e.registry, &e.functions, &d.id.path).unwrap();
    assert_eq!(sig.input("wetness").unwrap().range, Some((0.2, 5.0)));
    assert_eq!(sig.input("base_color").unwrap().range, None, "Float3 input");
}

/// Review Focus 5: every sample function file (FunctionInputs without `min` / `max`) loads and saves byte-identical.
#[test]
fn function_files_unchanged() {
    let sample = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/sample_game");
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"g\"\n").unwrap();
    std::fs::create_dir_all(dir.path().join("assets")).unwrap();
    let p = Project::open(dir.path()).unwrap();
    let mut files = Vec::new();
    let mut stack = vec![sample.join("shaders")];
    while let Some(d) = stack.pop() {
        for entry in std::fs::read_dir(&d).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else if path.to_string_lossy().ends_with(".function.ron") {
                files.push(path);
            }
        }
    }
    assert!(files.len() >= 4, "{files:?}");
    for file in files {
        let rel = file.strip_prefix(sample.join("shaders")).unwrap().to_string_lossy().replace('\\', "/");
        let id = AssetId::new(AssetType::Function, rel.trim_end_matches(".function.ron"));
        let original = std::fs::read(&file).unwrap();
        std::fs::create_dir_all(id.file(&p).parent().unwrap()).unwrap();
        std::fs::write(id.file(&p), &original).unwrap();
        let asset = assets::load(&p, &id).unwrap();
        ops::save(&p, &id, &asset).unwrap();
        assert_eq!(std::fs::read(id.file(&p)).unwrap(), original, "{rel}");
    }
}

/// The comment frame `AddComment` added (the one not in `before`).
fn new_frame(e: &Editor, before: &[String]) -> shader_graph_core::model::Frame {
    e.doc().unwrap().asset.comments.iter().find(|f| !before.contains(&f.id)).cloned().expect("a new comment frame")
}

fn frame_ids(e: &Editor) -> Vec<String> {
    e.doc().unwrap().asset.comments.iter().map(|f| f.id.clone()).collect()
}

/// Design §6.1: with nothing selected, a comment is a 400x300 frame at the cursor snapped to the grid, around no
/// node (not around the whole graph), in one undo step.
#[test]
fn add_comment_without_selection_is_default_size_at_cursor() {
    let (_dir, mut e) = project();
    open_rock_wet(&mut e);
    assert!(e.doc().unwrap().asset.nodes.len() >= 5);
    e.apply(Action::Select(BTreeSet::new()));
    let before = frame_ids(&e);
    e.apply(Action::AddComment(Some((12.3, -7.6))));
    let f = new_frame(&e, &before);
    assert_eq!(f.rect, (10.0, -10.0, 400.0, 300.0));
    assert!(f.nodes.is_empty(), "{:?}", f.nodes);
    e.apply(Action::Undo);
    assert_eq!(frame_ids(&e), before, "one undo step");
    assert!(!e.is_dirty());
    // Without a cursor: at the origin.
    e.apply(Action::AddComment(None));
    assert_eq!(new_frame(&e, &before).rect, (0.0, 0.0, 400.0, 300.0));
}

/// A comment with a selection wraps the selected nodes; the cursor does not matter.
#[test]
fn add_comment_with_selection_wraps_it() {
    let (_dir, mut e) = project();
    open_rock_wet(&mut e);
    let a = add(&mut e, "stdlib::Constant");
    e.apply(Action::AddNode { kind: "stdlib::Time".into(), pos: (-500.0, 200.0), link: None, params: Vec::new() });
    let b = e.doc().unwrap().asset.nodes.iter().find(|(_, n)| n.kind == "stdlib::Time").unwrap().0.clone();
    let (pa, pb) = {
        let d = e.doc().unwrap();
        (d.asset.nodes[&a].pos, d.asset.nodes[&b].pos)
    };
    e.apply(Action::Select([a.clone(), b.clone()].into()));
    let before = frame_ids(&e);
    e.apply(Action::AddComment(Some((5000.0, 5000.0))));
    let f = new_frame(&e, &before);
    let (x0, y0, x1, y1) = (pa.0.min(pb.0), pa.1.min(pb.1), pa.0.max(pb.0), pa.1.max(pb.1));
    assert_eq!(f.rect, (x0 - 30.0, y0 - 50.0, x1 - x0 + 260.0, y1 - y0 + 220.0));
    let mut members = vec![a, b];
    members.sort();
    assert_eq!(f.nodes, members);
}

/// Design §5: the statusbar counts wires and comments too (the sidebar's "Info" subpane is gone).
#[test]
fn graph_status_counts_wires_and_comments() {
    let (_dir, mut e) = project();
    open_rock_wet(&mut e);
    let counts = e.status_info().unwrap().counts;
    let a = &e.doc().unwrap().asset;
    let (nodes, wires, comments) = (a.nodes.len(), a.edges.len(), a.comments.len());
    for want in [
        crate::ui::plural(nodes, "node", "nodes"),
        crate::ui::plural(wires, "wire", "wires"),
        crate::ui::plural(comments, "comment", "comments"),
    ] {
        assert!(counts.contains(&want), "{want} in {counts:?}");
    }
    e.apply(Action::Select(BTreeSet::new()));
    e.apply(Action::AddComment(None));
    let want = crate::ui::plural(comments + 1, "comment", "comments");
    assert!(e.status_info().unwrap().counts.contains(&want));
}

// ---- gradient collections (gradient design §8) ----

use crate::gradients_doc::{GradientSel, GradientsDoc};
use shader_graph_core::gradient::{MAX_STOPS, SLOTS, Slot, Stop};

/// The test project with a collection `common/ramps` on disk, opened in a tab.
fn gradients_project() -> (tempfile::TempDir, Editor, AssetId) {
    let (dir, mut e) = project();
    let id = ops::create_gradients(e.project.as_ref().unwrap(), "common/ramps").unwrap();
    e.rescan();
    e.apply(Action::OpenAsset(id.clone()));
    (dir, e, id)
}

fn gdoc(e: &Editor) -> &GradientsDoc {
    e.gradients_doc().expect("an active gradients tab")
}

fn stops(e: &Editor) -> Vec<Stop> {
    let d = gdoc(e);
    d.collection.slots[d.slot].stops.clone()
}

fn ga(e: &mut Editor, a: GradientsAction) {
    e.apply(Action::Gradients(a));
}

#[test]
fn gradients_create_open_save_reload() {
    let (dir, mut e) = project();
    e.apply(Action::NewAssetDialog(NewKind::Gradients));
    let Some(Dialog::Input { initial, title, .. }) = &e.dialog else { panic!("no dialog") };
    assert_eq!(initial, "new_gradients");
    assert_eq!(title, "New Gradient Collection");
    e.apply(Action::Submit("new_gradients".into()));
    assert!(dir.path().join("shaders/new_gradients.gradients.ron").is_file());
    let id = AssetId::new(AssetType::Gradients, "new_gradients");
    assert_eq!(e.asset_index(&id), e.active);
    assert_eq!(e.tab().unwrap().doc().view(), ViewKind::Gradients);
    assert_eq!(e.tab().unwrap().doc().watermark(), Some("gradients"));
    assert_eq!(gdoc(&e).collection.slots.len(), SLOTS);
    assert!(gdoc(&e).collection.slots.iter().all(Slot::is_empty));
    assert!(!e.is_dirty());
    assert!(e.gradient_collections().contains(&("new_gradients".to_string(), "new_gradients".to_string())));

    ga(&mut e, GradientsAction::AddStop(0.25));
    assert!(e.is_dirty());
    // The unsaved state wins over the file for previews.
    assert_eq!(e.gradient_collection("new_gradients").unwrap().slots[0].stops.len(), 1);
    e.apply(Action::Save);
    assert!(!e.is_dirty());
    let on_disk = assets::load_gradients(e.project.as_ref().unwrap(), &id).unwrap();
    assert_eq!(on_disk, gdoc(&e).collection);

    // Changed outside the editor: a clean tab reloads.
    let mut changed = on_disk.clone();
    changed.slots[1].name = "sky".into();
    ops::save_gradients(e.project.as_ref().unwrap(), &id, &changed).unwrap();
    let i = e.asset_index(&id).unwrap();
    assert!(e.tabs[i].doc().changed_on_disk(e.project.as_ref().unwrap()));
    e.apply(Action::Reload(id.clone()));
    assert_eq!(gdoc(&e).collection, changed);
    assert!(!e.is_dirty());
}

#[test]
fn gradients_stop_edits_and_undo() {
    let (_dir, mut e, _) = gradients_project();
    for t in [0.0, 1.0, 0.5] {
        ga(&mut e, GradientsAction::AddStop(t));
    }
    let ts: Vec<f32> = stops(&e).iter().map(|s| s.t).collect();
    assert_eq!(ts, vec![0.0, 0.5, 1.0]);
    assert_eq!(gdoc(&e).selection, Some(GradientSel::Stop(1)));
    // An empty slot's first stop is opaque white; later ones sample the gradient.
    assert!(stops(&e).iter().all(|s| s.color == [1.0; 4]));

    // A drag of stop 1 is one undo step.
    let before = stops(&e);
    ga(&mut e, GradientsAction::SetStopT(1, 0.6, Some("drag".into())));
    ga(&mut e, GradientsAction::SetStopT(1, 0.7, Some("drag".into())));
    assert_eq!(stops(&e)[1].t, 0.7);
    e.apply(Action::CommitInline);
    e.apply(Action::Undo);
    assert_eq!(stops(&e), before);

    // Dragging a stop past its neighbour re-sorts; the selection follows it.
    ga(&mut e, GradientsAction::SetStopColor(0, [1.0, 0.0, 0.0, 1.0], None));
    ga(&mut e, GradientsAction::SetStopT(0, 0.7, Some("drag".into())));
    assert_eq!(gdoc(&e).selection, Some(GradientSel::Stop(1)));
    assert_eq!(stops(&e)[1].color, [1.0, 0.0, 0.0, 1.0]);
    assert_eq!(stops(&e)[1].t, 0.7);
    e.apply(Action::CommitInline);
    e.apply(Action::Undo);
    e.apply(Action::Undo);
    assert_eq!(stops(&e), before);

    // Midpoints are clamped and belong to their stop.
    ga(&mut e, GradientsAction::SetMid(0, 5.0, None));
    assert_eq!(stops(&e)[0].mid, shader_graph_core::gradient::MID_MAX);
    e.apply(Action::Undo);

    ga(&mut e, GradientsAction::Select(Some(GradientSel::Stop(1))));
    e.apply(Action::DeleteSelection);
    assert_eq!(stops(&e).len(), 2);
    assert_eq!(gdoc(&e).selection, None);
    e.apply(Action::Undo);
    assert_eq!(stops(&e), before);
    e.apply(Action::Redo);
    assert_eq!(stops(&e).len(), 2);
    // An out-of-range selection is refused.
    ga(&mut e, GradientsAction::Select(Some(GradientSel::Mid(1))));
    assert_eq!(gdoc(&e).selection, None);
}

/// Gradient design §8: the color picker's changes during one pick (key `"color"`) are one undo step; the picker's
/// `is_final` or its closing (`CommitInline`) starts the next.
#[test]
fn gradients_color_pick_is_one_undo_step() {
    let (_dir, mut e, _) = gradients_project();
    ga(&mut e, GradientsAction::AddStop(0.5));
    let original = stops(&e)[0].color;
    for c in [[0.2, 0.0, 0.0, 1.0], [0.4, 0.1, 0.0, 1.0], [0.6, 0.2, 0.1, 0.5]] {
        ga(&mut e, GradientsAction::SetStopColor(0, c, Some("color".into())));
    }
    assert_eq!(stops(&e)[0].color, [0.6, 0.2, 0.1, 0.5]);
    e.apply(Action::CommitInline);
    ga(&mut e, GradientsAction::SetStopColor(0, [0.0, 0.0, 1.0, 1.0], Some("color".into())));
    e.apply(Action::CommitInline);
    e.apply(Action::Undo);
    assert_eq!(stops(&e)[0].color, [0.6, 0.2, 0.1, 0.5], "the second pick is its own step");
    e.apply(Action::Undo);
    assert_eq!(stops(&e)[0].color, original, "the first pick is one step");
    assert_eq!(stops(&e).len(), 1, "the stop itself stays");
}

#[test]
fn gradients_stop_limit() {
    let (_dir, mut e, _) = gradients_project();
    for k in 0..MAX_STOPS {
        ga(&mut e, GradientsAction::AddStop(k as f32 / MAX_STOPS as f32));
    }
    assert_eq!(stops(&e).len(), MAX_STOPS);
    let logged = e.messages.len();
    ga(&mut e, GradientsAction::AddStop(0.5));
    assert_eq!(stops(&e).len(), MAX_STOPS);
    assert_eq!(e.messages.len(), logged + 1);
    assert!(e.messages.last().unwrap().text.contains("at most 32 stops"), "{:?}", e.messages.last());
    let mut d = gdoc(&e).clone();
    assert!(!d.add_stop(0.5));
}

#[test]
fn gradients_slots_move_clear_copy_paste() {
    let (_dir, mut e, _) = gradients_project();
    let ids: Vec<u32> = gdoc(&e).collection.slots.iter().map(|s| s.id).collect();
    ga(&mut e, GradientsAction::AddStop(0.3));
    // Typing the name is one undo step.
    for name in ["g", "gr", "grass"] {
        let (target, value) = (PropTarget::GradientName, Value::Str(name.into()));
        e.apply(Action::SetValue { target, value, merge: true });
    }
    e.apply(Action::Undo);
    assert_eq!(gdoc(&e).collection.slots[0].name, "");
    e.apply(Action::Redo);
    let slot0 = gdoc(&e).collection.slots[0].clone();
    assert_eq!(slot0.name, "grass");

    // Moving keeps the id with its content; the selection follows the moved slot.
    ga(&mut e, GradientsAction::MoveSlot(0, 4));
    assert_eq!(gdoc(&e).collection.slots[4], slot0);
    assert_eq!(gdoc(&e).slot, 4);
    assert_eq!(gdoc(&e).collection.slots[0].id, ids[1]);
    let moved: Vec<u32> = gdoc(&e).collection.slots.iter().map(|s| s.id).collect();
    assert_eq!(moved[..4], ids[1..5]);

    // Copy, paste into slot 7: stops and name, slot 7 keeps its id.
    e.apply(Action::Copy);
    ga(&mut e, GradientsAction::SelectSlot(7));
    assert_eq!(gdoc(&e).slot, 7);
    e.apply(Action::Paste(None));
    let s7 = gdoc(&e).collection.slots[7].clone();
    assert_eq!((s7.id, &s7.name, &s7.stops), (ids[7], &slot0.name, &slot0.stops));

    // Duplicate goes to the next empty slot after the selected one.
    e.apply(Action::Duplicate);
    assert_eq!(gdoc(&e).slot, 8);
    assert_eq!(gdoc(&e).collection.slots[8].stops, slot0.stops);
    assert_eq!(gdoc(&e).collection.slots[8].id, ids[8]);
    // From the last slot it wraps around (slot 0 is empty).
    ga(&mut e, GradientsAction::MoveSlot(8, SLOTS - 1));
    let mut d = gdoc(&e).clone();
    assert!(d.duplicate_slot());
    assert_eq!(d.slot, 0);

    // Clearing keeps the id.
    ga(&mut e, GradientsAction::SelectSlot(7));
    ga(&mut e, GradientsAction::ClearSlot);
    let s7 = &gdoc(&e).collection.slots[7];
    assert!(s7.is_empty() && s7.name.is_empty());
    assert_eq!(s7.id, ids[7]);
    e.apply(Action::Undo);
    assert_eq!(gdoc(&e).collection.slots[7].stops, slot0.stops);

    // No empty slot left: duplicate refuses.
    let mut full = gdoc(&e).clone();
    for s in &mut full.collection.slots {
        s.stops = slot0.stops.clone();
    }
    assert!(!full.duplicate_slot());
}

#[test]
fn gradients_tab_restored_from_config_key() {
    let (dir, mut e, id) = gradients_project();
    let key = e.tab().unwrap().doc().config_key().unwrap();
    assert_eq!(key, "shaders/common/ramps.gradients.ron");
    assert_eq!(parse_config_key(&key), Some(TabKey::Asset(id.clone())));
    e.save_tabs_to_config();
    let mut next = Editor::with_config(e.config.clone(), None);
    next.open_project(dir.path()).unwrap();
    assert_eq!(next.gradients_doc().map(|d| &d.id), Some(&id));
}

/// Open graphs follow a renamed / deleted collection (gradient design §6), as for curves and textures: a clean
/// document reloads the rewritten file, a dirty one gets the change as an unsaved edit; delete also zeroes the slot.
#[test]
fn gradients_rename_and_delete_follow_open_graphs() {
    use shader_graph_core::registry::gradient_refs;
    use shader_stdlib::nodes::color::{GRADIENT_COLLECTION_PARAM, GRADIENT_SLOT_PARAM, SAMPLE_GRADIENT_KIND};
    let (_dir, mut e, id) = gradients_project();
    let slot_id = gdoc(&e).collection.slots[3].id as i32;
    let rock = AssetId::new(AssetType::Material, "env/rock_wet");
    open_rock_wet(&mut e);
    let node = add(&mut e, SAMPLE_GRADIENT_KIND);
    let set = |key: &str, value: Value| Action::SetValue {
        target: PropTarget::Param { node: node.clone(), key: key.into() },
        value,
        merge: false,
    };
    e.apply(set(GRADIENT_COLLECTION_PARAM, Value::Str("common/ramps".into())));
    e.apply(set(GRADIENT_SLOT_PARAM, Value::Int(slot_id)));
    e.apply(Action::Save);
    let doc = |e: &Editor| e.tabs[e.doc_index(&rock).unwrap()].graph().unwrap().clone();
    let refs = |e: &Editor| gradient_refs(&doc(e).asset, &e.registry).into_iter().map(|r| r.2).collect::<Vec<_>>();
    let params = |e: &Editor| {
        let n = &doc(e).asset.nodes[&node];
        (n.params[GRADIENT_COLLECTION_PARAM].clone(), n.params[GRADIENT_SLOT_PARAM].clone())
    };

    // Clean: the rewritten file is reloaded.
    e.apply(Action::RenameDialog(id.clone()));
    e.apply(Action::Submit("env/terrain".into()));
    let moved = AssetId::new(AssetType::Gradients, "env/terrain");
    assert!(e.asset_index(&moved).is_some(), "the gradients tab follows");
    assert_eq!(refs(&e), vec!["env/terrain"]);
    assert!(!doc(&e).is_dirty());

    // Dirty: the open material gets the new id as an unsaved edit, the slot is kept.
    let i = e.doc_index(&rock).unwrap();
    e.tabs[i].graph_mut().unwrap().edit(|a| a.nodes.get_mut(&node).unwrap().pos.0 += 10.0);
    e.apply(Action::RenameDialog(moved.clone()));
    e.apply(Action::Submit("env/ground".into()));
    let moved = AssetId::new(AssetType::Gradients, "env/ground");
    assert!(doc(&e).is_dirty(), "unsaved edits stay unsaved");
    assert_eq!(params(&e), (Value::Str("env/ground".into()), Value::Int(slot_id)));

    e.apply(Action::Delete(moved.clone()));
    assert!(e.asset_index(&moved).is_none());
    assert!(refs(&e).is_empty());
    assert!(doc(&e).is_dirty(), "unsaved edits stay unsaved");
    assert_eq!(params(&e), (Value::Str(String::new()), Value::Int(0)));
}

/// Gradient design §6: picking another collection resets the slot (its ids mean nothing there) in the same undo
/// step; the node's "Open gradients" target is the collection.
#[test]
fn gradient_node_props_reset_slot_on_collection_change() {
    use shader_stdlib::nodes::color::{GRADIENT_COLLECTION_PARAM, GRADIENT_SLOT_PARAM, SAMPLE_GRADIENT_KIND};
    let (_dir, mut e, id) = gradients_project();
    let slot_id = gdoc(&e).collection.slots[3].id as i32;
    let sky = ops::create_gradients(e.project.as_ref().unwrap(), "common/sky").unwrap();
    e.rescan();
    open_rock_wet(&mut e);
    let node = add(&mut e, SAMPLE_GRADIENT_KIND);
    let set = |key: &str, value: Value| Action::SetValue {
        target: PropTarget::Param { node: node.clone(), key: key.into() },
        value,
        merge: false,
    };
    let params = |e: &Editor| (param_of(e, &node, GRADIENT_COLLECTION_PARAM), param_of(e, &node, GRADIENT_SLOT_PARAM));
    e.apply(set(GRADIENT_COLLECTION_PARAM, Value::Str("common/ramps".into())));
    e.apply(set(GRADIENT_SLOT_PARAM, Value::Int(slot_id)));
    assert_eq!(e.referenced_asset(&node), Some(id));
    let before = params(&e);
    assert_eq!(before, (Some(Value::Str("common/ramps".into())), Some(Value::Int(slot_id))));

    e.apply(set(GRADIENT_COLLECTION_PARAM, Value::Str("common/sky".into())));
    assert_eq!(params(&e), (Some(Value::Str("common/sky".into())), Some(Value::Int(0))));
    assert_eq!(e.referenced_asset(&node), Some(sky));
    e.apply(Action::Undo);
    assert_eq!(params(&e), before, "one undo step restores both");

    // Another param of the node leaves the slot alone.
    e.apply(set(GRADIENT_SLOT_PARAM, Value::Int(slot_id)));
    e.apply(set(GRADIENT_COLLECTION_PARAM, Value::Str("common/missing".into())));
    assert_eq!(e.referenced_asset(&node), None, "a missing collection opens nothing");
}

/// Gradient design §6: only a real collection change resets the slot — setting or dropping the node's own collection
/// keeps the picked gradient; another collection resets it, one undo step restores both.
#[test]
fn gradient_slot_kept_when_collection_unchanged() {
    use crate::droppable::Dropped;
    use shader_stdlib::nodes::color::{GRADIENT_COLLECTION_PARAM, GRADIENT_SLOT_PARAM, SAMPLE_GRADIENT_KIND};
    let (_dir, mut e, _id) = gradients_project();
    let slot_id = gdoc(&e).collection.slots[3].id as i32;
    ops::create_gradients(e.project.as_ref().unwrap(), "common/sky").unwrap();
    e.rescan();
    open_rock_wet(&mut e);
    let node = add(&mut e, SAMPLE_GRADIENT_KIND);
    let set = |key: &str, value: Value| Action::SetValue {
        target: PropTarget::Param { node: node.clone(), key: key.into() },
        value,
        merge: false,
    };
    let params = |e: &Editor| (param_of(e, &node, GRADIENT_COLLECTION_PARAM), param_of(e, &node, GRADIENT_SLOT_PARAM));
    e.apply(set(GRADIENT_COLLECTION_PARAM, Value::Str("common/ramps".into())));
    e.apply(set(GRADIENT_SLOT_PARAM, Value::Int(slot_id)));
    let picked = (Some(Value::Str("common/ramps".into())), Some(Value::Int(slot_id)));
    assert_eq!(params(&e), picked);

    e.apply(set(GRADIENT_COLLECTION_PARAM, Value::Str("common/ramps".into())));
    assert_eq!(params(&e), picked, "setting the same collection keeps the slot");
    let item = Dropped::Gradients("common/ramps".into());
    e.apply(Action::Drop { item, pos: (0.0, 0.0), node: Some(node.clone()) });
    assert_eq!(params(&e), picked, "dropping the node's own collection keeps the slot");

    let item = Dropped::Gradients("common/sky".into());
    e.apply(Action::Drop { item, pos: (0.0, 0.0), node: Some(node.clone()) });
    assert_eq!(params(&e), (Some(Value::Str("common/sky".into())), Some(Value::Int(0))));
    e.apply(Action::Undo);
    assert_eq!(params(&e), picked, "one undo step restores both");
}

/// Gradient design §6: a collection dropped onto a graph adds a Sample Gradient reading it, no gradient picked; onto a
/// Sample Gradient it replaces the collection and resets the slot.
#[test]
fn gradient_drop_creates_sample_gradient() {
    use crate::droppable::Dropped;
    use crate::ui::tree::{TreeItem, TreeRow};
    use shader_stdlib::nodes::color::{GRADIENT_COLLECTION_PARAM, GRADIENT_SLOT_PARAM, SAMPLE_GRADIENT_KIND};
    let (_dir, mut e, id) = gradients_project();
    let slot_id = gdoc(&e).collection.slots[3].id as i32;
    let row = TreeRow { tree: TreeId::Project, item: TreeItem::Asset(id.clone()) };
    assert_eq!(Dropped::from_row(&row), Some(Dropped::Gradients("common/ramps".into())));

    open_rock_wet(&mut e);
    let item = Dropped::Gradients("common/ramps".into());
    e.apply(Action::Drop { item, pos: (21.0, 9.0), node: None });
    let added = nodes_of(&e, SAMPLE_GRADIENT_KIND);
    assert_eq!(added.len(), 1);
    let node = added[0].clone();
    assert_eq!(e.doc().unwrap().asset.nodes[&node].pos, (20.0, 10.0));
    assert_eq!(param_of(&e, &node, GRADIENT_COLLECTION_PARAM), Some(Value::Str("common/ramps".into())));
    let slot = e.registry.resolved_params(SAMPLE_GRADIENT_KIND, &e.doc().unwrap().asset.nodes[&node].params);
    assert_eq!(slot.get(GRADIENT_SLOT_PARAM), Some(&Value::Int(0)), "no gradient picked");
    assert_eq!(e.doc().unwrap().selection, [node.clone()].into());
    assert_eq!(e.referenced_asset(&node), Some(id));

    // Onto the node: the collection is replaced and the slot reset.
    ops::create_gradients(e.project.as_ref().unwrap(), "common/sky").unwrap();
    e.rescan();
    let target = PropTarget::Param { node: node.clone(), key: GRADIENT_SLOT_PARAM.into() };
    e.apply(Action::SetValue { target, value: Value::Int(slot_id), merge: false });
    let count = e.doc().unwrap().asset.nodes.len();
    let item = Dropped::Gradients("common/sky".into());
    e.apply(Action::Drop { item, pos: (0.0, 0.0), node: Some(node.clone()) });
    assert_eq!(e.doc().unwrap().asset.nodes.len(), count, "no new node");
    assert_eq!(param_of(&e, &node, GRADIENT_COLLECTION_PARAM), Some(Value::Str("common/sky".into())));
    assert_eq!(param_of(&e, &node, GRADIENT_SLOT_PARAM), Some(Value::Int(0)));
}

/// Gradient design §6: the node preview shows the selected slot's baked row, of the open collection tab's unsaved
/// state; no collection or no gradient picked shows "n/a".
#[test]
fn gradient_preview_key_follows_open_tab() {
    use crate::ui::gradient_preview::GradientPreview;
    use crate::ui::node_preview::NodePreview;
    use shader_stdlib::nodes::color::{GRADIENT_COLLECTION_PARAM, GRADIENT_SLOT_PARAM, SAMPLE_GRADIENT_KIND};
    let (_dir, mut e, id) = gradients_project();
    ga(&mut e, GradientsAction::AddStop(0.2));
    ga(&mut e, GradientsAction::AddStop(0.8));
    let slot_id = gdoc(&e).collection.slots[0].id as i32;
    let row = gdoc(&e).collection.slots[0].bake_row();
    open_rock_wet(&mut e);
    let graph = e.active.unwrap();
    let node = add(&mut e, SAMPLE_GRADIENT_KIND);
    let key = |e: &Editor| GradientPreview::key(e, e.doc().unwrap(), &node);
    let set = |key: &str, value: Value| Action::SetValue {
        target: PropTarget::Param { node: node.clone(), key: key.into() },
        value,
        merge: false,
    };
    assert_eq!(key(&e), None, "no collection: n/a");
    e.apply(set(GRADIENT_COLLECTION_PARAM, Value::Str("common/ramps".into())));
    assert_eq!(key(&e), None, "no gradient: n/a");
    e.apply(set(GRADIENT_SLOT_PARAM, Value::Int(slot_id)));
    assert_eq!(key(&e), Some(row));

    // An unsaved stop color in the open collection tab shows on the node.
    e.apply(Action::ActivateTab(e.asset_index(&id).unwrap()));
    ga(&mut e, GradientsAction::SetStopColor(0, [1.0, 0.0, 0.0, 1.0], None));
    assert!(gdoc(&e).is_dirty());
    let edited = gdoc(&e).collection.slots[0].bake_row();
    assert_ne!(edited, row);
    e.apply(Action::ActivateTab(graph));
    assert_eq!(key(&e), Some(edited));

    e.apply(set(GRADIENT_SLOT_PARAM, Value::Int(12345)));
    assert_eq!(key(&e), None, "a slot id not in the collection: n/a");
}

/// Decision 0038: Ctrl+V / Ctrl+D of a Named Route declaration insert its usage in the same graph; another graph
/// gets a duplicate (a usage there would dangle).
#[test]
fn route_declaration_pastes_as_usage() {
    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet")));
    let decl = add(&mut e, "stdlib::NamedRoute");
    let kinds = |e: &Editor, kind: &str| e.doc().unwrap().asset.nodes.values().filter(|n| n.kind == kind).count();
    let selected_route = |e: &Editor| {
        let d = e.doc().unwrap();
        assert_eq!(d.selection.len(), 1);
        let n = &d.asset.nodes[d.selection.first().unwrap()];
        assert_eq!(n.kind, "stdlib::NamedRouteUsage");
        n.params.get(shader_graph_core::registry::ROUTE_PARAM).cloned()
    };
    e.apply(Action::Select([decl.clone()].into()));
    e.apply(Action::Duplicate);
    assert_eq!(selected_route(&e), Some(Value::Str(decl.clone())));
    assert_eq!(kinds(&e, "stdlib::NamedRoute"), 1);
    e.apply(Action::Undo);
    assert_eq!(kinds(&e, "stdlib::NamedRouteUsage"), 0, "one undo step");

    e.apply(Action::Select([decl.clone()].into()));
    e.apply(Action::Copy);
    e.apply(Action::Paste(None));
    assert_eq!(selected_route(&e), Some(Value::Str(decl.clone())));
    assert_eq!(kinds(&e, "stdlib::NamedRoute"), 1);

    // Another graph tab: the declaration is duplicated.
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "fx/hologram")));
    assert_eq!(active_asset(&e), AssetId::new(AssetType::Material, "fx/hologram"));
    let before = kinds(&e, "stdlib::NamedRoute");
    e.apply(Action::Paste(None));
    assert_eq!(kinds(&e, "stdlib::NamedRoute"), before + 1);
    assert_eq!(kinds(&e, "stdlib::NamedRouteUsage"), 0);
}

// ---- navigation history (decision 0041) ----

fn nav_a() -> AssetId {
    AssetId::new(AssetType::Material, "env/rock_wet")
}

fn nav_b() -> AssetId {
    AssetId::new(AssetType::Material, "fx/hologram")
}

/// Opens `env/rock_wet`, `fx/hologram` and the curve `common/pulse`, in that order.
fn nav_open_abc(e: &mut Editor) {
    for id in [nav_a(), nav_b(), pulse()] {
        e.apply(Action::OpenAsset(id));
    }
}

fn key(id: &AssetId) -> TabKey {
    TabKey::Asset(id.clone())
}

fn nav_tabs(e: &Editor) -> Vec<TabKey> {
    e.nav.entries().iter().map(|n| n.tab.clone()).collect()
}

#[test]
fn nav_records_tab_switches() {
    let (_dir, mut e) = project();
    nav_open_abc(&mut e);
    assert_eq!(nav_tabs(&e), vec![key(&nav_a()), key(&nav_b()), key(&pulse())]);
    e.apply(Action::NavBack);
    assert_eq!(e.active_key(), Some(key(&nav_b())));
    e.apply(Action::NavBack);
    assert_eq!(e.active_key(), Some(key(&nav_a())));
    e.apply(Action::NavForward);
    assert_eq!(e.active_key(), Some(key(&nav_b())));
    assert_eq!(e.nav.entries().len(), 3);
    assert!(!e.is_dirty());
}

#[test]
fn nav_back_reopens_closed_tab() {
    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(nav_a()));
    e.apply(Action::OpenAsset(nav_b()));
    e.apply(Action::CloseTab(usize::MAX));
    assert_eq!(e.active_key(), Some(key(&nav_a())));
    assert_eq!(nav_tabs(&e), vec![key(&nav_a()), key(&nav_b()), key(&nav_a())]);
    e.apply(Action::NavBack);
    assert_eq!(e.active_key(), Some(key(&nav_b())));
    assert_eq!(e.tabs.len(), 2);
    assert_eq!(e.active, Some(1), "a reopened tab is appended");
}

#[test]
fn nav_back_reopens_last_closed_tab() {
    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(nav_a()));
    e.apply(Action::CloseTab(usize::MAX));
    assert_eq!(e.active, None);
    e.apply(Action::NavBack);
    assert_eq!(e.active_key(), Some(key(&nav_a())));
    assert_eq!(e.tabs.len(), 1);
}

#[test]
fn nav_skips_deleted_and_drops_it() {
    let (_dir, mut e) = project();
    nav_open_abc(&mut e);
    e.apply(Action::Delete(nav_b()));
    assert!(!e.disk.contains(&nav_b()));
    assert_eq!(e.active_key(), Some(key(&pulse())));
    e.apply(Action::NavBack);
    assert_eq!(e.active_key(), Some(key(&nav_a())));
    e.apply(Action::NavBack);
    assert_eq!(e.active_key(), Some(key(&nav_a())), "nothing further back");
    assert_eq!(nav_tabs(&e), vec![key(&nav_a()), key(&pulse())]);
}

#[test]
fn nav_skips_unopenable_keeps_it() {
    let (_dir, mut e) = project();
    nav_open_abc(&mut e);
    // B's tab is closed (otherwise Back just activates it), then its file breaks.
    e.apply(Action::CloseTab(e.asset_index(&nav_b()).unwrap()));
    let file = e.project.as_ref().unwrap().root.join(nav_b().source_rel());
    std::fs::write(&file, "not ron (").unwrap();
    e.rescan();
    assert!(e.disk.contains(&nav_b()));
    e.apply(Action::NavBack);
    assert_eq!(e.active_key(), Some(key(&nav_a())));
    assert!(nav_tabs(&e).contains(&key(&nav_b())), "kept: the file may be fixed");
    assert!(e.dialog.is_none(), "the walk shows no error dialog, the console has the error");
    assert!(e.messages.iter().any(|m| m.level == Level::Error && m.text.contains("cannot open")));
}

#[test]
fn nav_open_tab_of_vanished_file_is_not_gone() {
    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(nav_a()));
    e.apply(Action::OpenAsset(nav_b()));
    // A's file disappears behind the editor's back; its tab stays open.
    let file = e.project.as_ref().unwrap().root.join(nav_a().source_rel());
    std::fs::remove_file(&file).unwrap();
    e.rescan();
    assert!(!e.disk.contains(&nav_a()));
    assert!(e.asset_index(&nav_a()).is_some());
    e.apply(Action::NavBack);
    assert_eq!(e.active_key(), Some(key(&nav_a())));
    assert_eq!(nav_tabs(&e), vec![key(&nav_a()), key(&nav_b())], "the open tab's entry is kept");
}

#[test]
fn nav_end_of_list_logs_nothing() {
    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(nav_a()));
    let before = e.messages.len();
    e.apply(Action::NavBack);
    e.apply(Action::NavForward);
    assert_eq!(e.active_key(), Some(key(&nav_a())));
    assert_eq!(e.messages.len(), before, "no message at an end of the list");
}

#[test]
fn nav_follows_rename() {
    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(nav_a()));
    e.apply(Action::OpenAsset(nav_b()));
    e.apply(Action::RenameDialog(nav_a()));
    e.apply(Action::Submit("env/rock_dry".into()));
    let moved = AssetId::new(AssetType::Material, "env/rock_dry");
    assert!(e.disk.contains(&moved));
    e.apply(Action::NavBack);
    assert_eq!(e.active_key(), Some(key(&moved)));
}

#[test]
fn nav_navigation_records_nothing() {
    let (_dir, mut e) = project();
    nav_open_abc(&mut e);
    e.apply(Action::NavBack);
    e.apply(Action::NavBack);
    assert_eq!(e.nav.entries().len(), 3);
    assert_eq!(e.nav.current().map(|n| &n.tab), Some(&key(&nav_a())));
}

#[test]
fn nav_cleared_on_project_open() {
    let (dir, mut e) = project();
    e.apply(Action::OpenAsset(nav_a()));
    e.apply(Action::OpenAsset(nav_b()));
    e.open_project(dir.path()).unwrap();
    assert_eq!(nav_tabs(&e), vec![key(&nav_b())], "only the restored active tab");
    e.close_project(Unsaved::Discard);
    assert!(e.nav.entries().is_empty());
}

#[test]
fn nav_back_restores_view_request() {
    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(nav_a()));
    let view = NavView { centre: bevy::math::Vec2::new(10.0, 20.0), zoom: 0.5 };
    e.nav.note_view(&key(&nav_a()), view, 600.0);
    e.apply(Action::OpenAsset(nav_b()));
    e.apply(Action::NavBack);
    assert_eq!(e.active_key(), Some(key(&nav_a())));
    assert_eq!(e.view_requests, vec![ViewRequest::Set { centre: bevy::math::Vec2::new(10.0, 20.0), zoom: 0.5 }]);
}

#[test]
fn nav_ignored_while_dialog_open() {
    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(nav_a()));
    e.apply(Action::OpenAsset(nav_b()));
    e.dialog = Some(Dialog::About);
    e.apply(Action::NavBack);
    assert_eq!(e.active_key(), Some(key(&nav_b())));
}
