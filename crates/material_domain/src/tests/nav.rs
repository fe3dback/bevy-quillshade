//! Back / Forward (decision 0041) and the core paths through Material (save, close, preview, payload routing).

use super::*;

pub(super) fn nav_a() -> AssetId {
    AssetId::new(AssetType::Material, "env/rock_wet")
}

pub(super) fn nav_b() -> AssetId {
    AssetId::new(AssetType::Material, "fx/hologram")
}

/// Opens `env/rock_wet`, `fx/hologram` and the curve `common/pulse`, in that order.
pub(super) fn nav_open_abc(e: &mut Editor) {
    for id in [nav_a(), nav_b(), pulse()] {
        e.apply(Action::OpenAsset(id.to_ref()));
    }
}

pub(super) fn key(id: &AssetId) -> TabKey {
    crate::tab::tab_key(&MaterialKey::Asset(id.clone()))
}

pub(super) fn nav_tabs(e: &Editor) -> Vec<TabKey> {
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
    e.apply(Action::OpenAsset(nav_a().to_ref()));
    e.apply(Action::OpenAsset(nav_b().to_ref()));
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
    e.apply(Action::OpenAsset(nav_a().to_ref()));
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
    e.apply(Action::Delete(nav_b().to_ref()));
    assert!(!e.domains.material().disk.contains(&nav_b()));
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
    assert!(e.domains.material().disk.contains(&nav_b()));
    e.apply(Action::NavBack);
    assert_eq!(e.active_key(), Some(key(&nav_a())));
    assert!(nav_tabs(&e).contains(&key(&nav_b())), "kept: the file may be fixed");
    assert!(e.dialog.is_none(), "the walk shows no error dialog, the console has the error");
    assert!(e.messages.iter().any(|m| m.level == Level::Error && m.text.contains("cannot open")));
}

#[test]
fn nav_open_tab_of_vanished_file_is_not_gone() {
    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(nav_a().to_ref()));
    e.apply(Action::OpenAsset(nav_b().to_ref()));
    // A's file disappears behind the editor's back; its tab stays open.
    let file = e.project.as_ref().unwrap().root.join(nav_a().source_rel());
    std::fs::remove_file(&file).unwrap();
    e.rescan();
    assert!(!e.domains.material().disk.contains(&nav_a()));
    assert!(e.asset_index(&nav_a()).is_some());
    e.apply(Action::NavBack);
    assert_eq!(e.active_key(), Some(key(&nav_a())));
    assert_eq!(nav_tabs(&e), vec![key(&nav_a()), key(&nav_b())], "the open tab's entry is kept");
}

#[test]
fn nav_end_of_list_logs_nothing() {
    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(nav_a().to_ref()));
    let before = e.messages.len();
    e.apply(Action::NavBack);
    e.apply(Action::NavForward);
    assert_eq!(e.active_key(), Some(key(&nav_a())));
    assert_eq!(e.messages.len(), before, "no message at an end of the list");
}

#[test]
fn nav_follows_rename() {
    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(nav_a().to_ref()));
    e.apply(Action::OpenAsset(nav_b().to_ref()));
    e.apply(Action::RenameDialog(nav_a().to_ref()));
    e.apply(Action::Submit("env/rock_dry".into()));
    let moved = AssetId::new(AssetType::Material, "env/rock_dry");
    assert!(e.domains.material().disk.contains(&moved));
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
    e.apply(Action::OpenAsset(nav_a().to_ref()));
    e.apply(Action::OpenAsset(nav_b().to_ref()));
    e.open_project(dir.path()).unwrap();
    assert_eq!(nav_tabs(&e), vec![key(&nav_b())], "only the restored active tab");
    e.close_project(Unsaved::Discard);
    assert!(e.nav.entries().is_empty());
}

#[test]
fn nav_back_restores_view_request() {
    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(nav_a().to_ref()));
    let view = NavView { centre: bevy::math::Vec2::new(10.0, 20.0), zoom: 0.5 };
    e.nav.note_view(&key(&nav_a()), view, 600.0);
    e.apply(Action::OpenAsset(nav_b().to_ref()));
    e.apply(Action::NavBack);
    assert_eq!(e.active_key(), Some(key(&nav_a())));
    assert_eq!(e.view_requests, vec![ViewRequest::Set { centre: bevy::math::Vec2::new(10.0, 20.0), zoom: 0.5 }]);
}

#[test]
fn nav_ignored_while_dialog_open() {
    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(nav_a().to_ref()));
    e.apply(Action::OpenAsset(nav_b().to_ref()));
    e.dialog = Some(Dialog::About);
    e.apply(Action::NavBack);
    assert_eq!(e.active_key(), Some(key(&nav_b())));
}

/// Review Focus 1: an edit queued for a curve tab (a `CurveAction` from a curve widget) that arrives after the active
/// tab became a graph is dropped: no panic, no message, neither document changes.
#[test]
fn stale_doc_payload_is_ignored() {
    let (_dir, mut e) = project();
    let curve = AssetId::new(AssetType::Curve, "common/pulse");
    e.apply(Action::OpenAsset(curve.to_ref()));
    // A selection, so the nudge would move a point if it reached the curve.
    e.apply(Action::from(CurveEdit::new(CurveAction::Select([0].into()))));
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet").to_ref()));
    let c = e.curve_index(&curve).unwrap();
    let g = e.active.unwrap();
    let revisions = |e: &Editor| (e.tabs[c].doc().revision(), e.tabs[g].doc().revision());
    let (before, points, messages) = (revisions(&e), e.tab_of::<CurveDoc>(c).unwrap().curve.clone(), e.messages.len());

    e.apply(Action::Tab(TabAction::Doc(Payload::new(CurveEdit::new(CurveAction::Nudge((0.1, 0.0)))))));
    assert_eq!(revisions(&e), before);
    assert_eq!(e.tab_of::<CurveDoc>(c).unwrap().curve, points);
    assert!(!e.is_dirty());
    assert_eq!((e.messages.len(), &e.dialog), (messages, &None), "nothing the user sees");
}

/// Review Focus 2: undo merging compares property keys by value. Two merged writes built from two separate
/// `PropKey`s with equal contents are one undo step; `CommitInline` ends the step.
#[test]
fn prop_keys_merge_by_value() {
    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet").to_ref()));
    // `n05` is the `wetness` Scalar parameter; its value param is `default`.
    let key = || PropKey::new(GraphProp::Param { node: "n05".into(), key: "default".into() });
    let set = |v: f32| Action::Tab(TabAction::SetValue { key: key(), value: PropValue::Float(v), merge: true });
    let original = e.current_value(&key());
    assert_eq!(original, Some(PropValue::Float(0.3)));
    e.apply(set(0.4));
    e.apply(set(0.5));
    e.apply(Action::Tab(TabAction::CommitInline));
    e.apply(set(0.6));
    assert_eq!(e.current_value(&key()), Some(PropValue::Float(0.6)));
    e.apply(Action::Tab(TabAction::Undo));
    assert_eq!(e.current_value(&key()), Some(PropValue::Float(0.5)), "the write after CommitInline is a step");
    e.apply(Action::Tab(TabAction::Undo));
    assert_eq!(e.current_value(&key()), original, "the two merged writes are one step");
    assert!(!e.is_dirty());
}

/// Undo reaches every editable kind through the core verb (design §6.2): each tab reverts its last edit.
#[test]
fn edit_verbs_reach_every_kind() {
    let (_dir, mut e) = project();
    let undo = |e: &mut Editor| e.apply(Action::Tab(TabAction::Undo));

    e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet").to_ref()));
    let nodes = e.doc().unwrap().asset.nodes.len();
    e.apply(MaterialAction::AddNode { kind: "stdlib::Time".into(), pos: (0.0, 0.0), link: None, params: Vec::new() });
    assert_eq!(e.doc().unwrap().asset.nodes.len(), nodes + 1);
    undo(&mut e);
    assert_eq!(e.doc().unwrap().asset.nodes.len(), nodes, "graph");

    e.apply(Action::OpenAsset(AssetId::new(AssetType::Curve, "common/pulse").to_ref()));
    let points = e.curve_doc().unwrap().curve.points.clone();
    e.apply(Action::from(CurveEdit::new(CurveAction::Add((0.33, 0.66)))));
    assert_ne!(e.curve_doc().unwrap().curve.points, points);
    undo(&mut e);
    assert_eq!(e.curve_doc().unwrap().curve.points, points, "curve");

    e.apply(Action::OpenAsset(AssetId::new(AssetType::Consts, "common/tuning").to_ref()));
    let amount = ConstsProp::Value("glow_amount".into());
    e.apply(set_value(amount.clone(), Value::Float(0.9), false));
    assert_eq!(shown(&e, &amount), Some(Value::Float(0.9)));
    undo(&mut e);
    assert_eq!(shown(&e, &amount), Some(Value::Float(0.35)), "constant table");

    e.apply(Action::OpenAsset(AssetId::new(AssetType::Texture, "textures/rock_albedo").to_ref()));
    let srgb = TextureKey::new(TextureProp::Field(TextureField::IsSrgb));
    let was = shown(&e, &srgb).unwrap();
    e.apply(set_value(srgb.clone(), Value::Bool(!was.as_bool()), false));
    assert_ne!(shown(&e, &srgb), Some(was.clone()));
    undo(&mut e);
    assert_eq!(shown(&e, &srgb), Some(was), "Texture asset");

    e.apply(Action::OpenAsset(AssetId::new(AssetType::Gradients, "common/terrain").to_ref()));
    let collection = e.gradients_doc().unwrap().collection.clone();
    e.apply(Action::from(GradientsAction::AddStop(0.42)));
    assert_ne!(e.gradients_doc().unwrap().collection, collection);
    undo(&mut e);
    assert_eq!(e.gradients_doc().unwrap().collection, collection, "gradient collection");
}

/// Review Focus 3: Save goes through core, which hands the saved tab to its domain (`DomainState::after_save`);
/// Material regenerates as before.
#[test]
fn save_regenerates_material() {
    let (_dir, mut e) = project();
    e.apply(Action::RegenAll);
    let wesl = rock_wet_wesl(&e);
    let before = std::fs::read_to_string(&wesl).unwrap();
    let lerp = add_lerp(&mut e);
    let out = output_node(&e);
    e.apply(GraphEdit::Connect { from: (lerp, "out".into()), to: (out, "roughness".into()) });
    assert!(e.is_dirty());
    assert_eq!(std::fs::read_to_string(&wesl).unwrap(), before, "an edit alone writes nothing");
    // A stale regeneration-only entry: a real regeneration rebuilds the map.
    let stale = AssetId::new(AssetType::Material, "gone/stale");
    e.domains.material_mut().regen_only.insert(stale.clone(), vec![Diagnostic::new(Code::E008, "stale")]);
    e.apply(Action::Save);
    assert_ne!(std::fs::read_to_string(&wesl).unwrap(), before, "saving regenerates the material");
    assert!(!e.domains.material().regen_only.contains_key(&stale), "regen_only is fresh");
    assert!(!e.tabs[e.active.unwrap()].doc().is_dirty());
}

#[test]
fn closing_dirty_tab_asks() {
    let (_dir, mut e) = project();
    add_lerp(&mut e);
    let i = e.active.unwrap();
    e.apply(Action::CloseTab(i));
    assert_eq!(e.tabs.len(), 1, "nothing closes yet");
    let Some(Dialog::Confirm { title, buttons, .. }) = &e.dialog else { panic!("{:?}", e.dialog) };
    assert_eq!(title, "Unsaved changes");
    let actions: Vec<&Action> = buttons.iter().map(|(_, a)| a).collect();
    assert_eq!(actions, [&Action::CloseDialog, &Action::DiscardTab(i), &Action::SaveAndCloseTab(i)]);
}

#[test]
fn preview_toggle_regenerates_from_memory() {
    let (dir, mut e) = project();
    e.apply(Action::RegenAll);
    let wesl = rock_wet_wesl(&e);
    let source = dir.path().join("quillshade/material/env/rock_wet.material.ron");
    let saved = std::fs::read(&source).unwrap();
    let lerp = add_lerp(&mut e);
    e.apply(MaterialAction::TogglePreview(lerp.clone()));
    assert!(std::fs::read_to_string(&wesl).unwrap().contains(&format!("// preview of node {lerp}")));
    assert_eq!(std::fs::read(&source).unwrap(), saved, "the .ron is not saved");
    assert!(e.is_dirty());
}

/// `Action::Domain` reaches the state of its domain (Material's `apply` hook); an unknown domain, or a payload the
/// domain does not know, changes nothing.
#[test]
fn domain_action_routes_to_its_state() {
    let (_dir, mut e) = project();
    let lerp = add_lerp(&mut e);
    e.apply(GraphEdit::Select(BTreeSet::new()));
    let select = GraphEdit::Select([lerp.clone()].into());
    e.apply(Action::Domain(MATERIAL, Payload::new(select.clone())));
    assert_eq!(e.doc().unwrap().selection, [lerp.clone()].into());
    // A Material command wraps as `Action::Domain`; a graph edit travels as an edit of the active tab (design §7.3),
    // which hands it to the same hook.
    let toggle = MaterialAction::TogglePreview(lerp.clone());
    assert_eq!(Action::from(toggle.clone()), Action::Domain(MATERIAL, Payload::new(toggle)));
    assert_eq!(Action::from(select), Action::Tab(TabAction::doc(GraphEdit::Select([lerp].into()))));
    let (revision, messages, props) = (e.doc().unwrap().revision, e.messages.len(), e.ui.props);
    e.apply(Action::Domain(DomainId::new("nope"), Payload::new(GraphEdit::Select(BTreeSet::new()))));
    e.apply(Action::Domain(MATERIAL, Payload::new(5u8)));
    assert_eq!((e.doc().unwrap().revision, e.messages.len(), e.ui.props), (revision, messages, props));
    assert!(!e.doc().unwrap().selection.is_empty() && e.dialog.is_none());
    assert!(e.domains.get::<MaterialState>().is_some(), "the state is back in place");
}

/// The mainbar's error count comes from the domains' `diagnostics` hook and matches the console/logs.
#[test]
fn diagnostics_hook_counts_like_the_console() {
    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet").to_ref()));
    e.apply(MaterialAction::AddNode { kind: "stdlib::Time".into(), pos: (0.0, 0.0), link: None, params: Vec::new() });
    // A regeneration-only error (E008) counts too.
    let id = AssetId::new(AssetType::Material, "env/rock_wet");
    e.domains.material_mut().regen_only.insert(id, vec![Diagnostic::new(Code::E008, "not ours")]);
    e.validate();
    let console = e.all_diagnostics();
    assert!(console.iter().any(Diagnostic::is_error));
    assert_eq!(e.error_count(), console.iter().filter(|d| d.is_error()).count());
    let hook = e.domains.get_dyn(MATERIAL).unwrap().diagnostics();
    assert_eq!(hook.len(), console.len());
    assert!(hook.iter().all(|d| d.domain == Some(MATERIAL)));
}

pub(super) fn regenerated_messages(e: &Editor) -> usize {
    e.messages.iter().filter(|m| m.text.starts_with("Regenerated")).count()
}

/// Save All on two dirty material tabs: Material's `after_save` runs once for the operation (one regeneration) and
/// writes both `.wesl`. With nothing dirty, Save All still regenerates once, as before.
#[test]
fn save_all_regenerates_every_saved_material_once() {
    let (_dir, mut e) = project();
    e.apply(Action::RegenAll);
    let ids = [AssetId::new(AssetType::Material, "env/rock_wet"), AssetId::new(AssetType::Material, "env/water")];
    let wesl = |e: &Editor, id: &AssetId| {
        std::fs::read_to_string(wesl_file(e.project.as_ref().unwrap(), id).unwrap()).unwrap()
    };
    let before: Vec<String> = ids.iter().map(|id| wesl(&e, id)).collect();
    for id in &ids {
        e.apply(Action::OpenAsset(id.to_ref()));
        e.apply(MaterialAction::AddNode {
            kind: "stdlib::Lerp".into(),
            pos: (-700.0, 0.0),
            link: None,
            params: Vec::new(),
        });
        let lerp = node_of(&e, "stdlib::Lerp");
        let out = output_node(&e);
        e.apply(GraphEdit::Connect { from: (lerp, "out".into()), to: (out, "roughness".into()) });
    }
    assert!(e.tabs.iter().all(|t| t.doc().is_dirty()));
    let regens = regenerated_messages(&e);
    e.apply(Action::SaveAll);
    assert_eq!(regenerated_messages(&e), regens + 1, "one regeneration for the whole Save All");
    for (id, old) in ids.iter().zip(&before) {
        assert_ne!(&wesl(&e, id), old, "{id:?} regenerated");
    }
    assert!(!e.is_dirty());
    e.apply(Action::SaveAll);
    assert_eq!(regenerated_messages(&e), regens + 2, "nothing dirty: Save All still regenerates");
}

/// A reload that fails (the file no longer parses) changes nothing: the preview's generated files stay, and the tab
/// stays open with its unsaved preview.
#[test]
fn failed_reload_keeps_preview_output() {
    let (dir, mut e) = project();
    e.apply(Action::RegenAll);
    let wesl = rock_wet_wesl(&e);
    let lerp = add_lerp(&mut e);
    e.apply(MaterialAction::TogglePreview(lerp.clone()));
    let previewed = std::fs::read_to_string(&wesl).unwrap();
    assert!(previewed.contains(&format!("// preview of node {lerp}")));
    let rock = AssetId::new(AssetType::Material, "env/rock_wet");
    std::fs::write(dir.path().join("quillshade/material/env/rock_wet.material.ron"), "not ( ron").unwrap();
    let previews = preview_messages(&e);
    e.apply(Action::Reload(asset_key(&rock)));
    assert_eq!(std::fs::read_to_string(&wesl).unwrap(), previewed, "the generated files stay");
    assert_eq!(preview_messages(&e), previews, "nothing regenerated");
    let i = e.doc_index(&rock).expect("the tab stays open");
    let d = e.tabs[i].get::<Doc>().unwrap();
    assert_eq!(d.asset.preview.node.as_deref(), Some(lerp.as_str()), "with its preview");
    assert!(d.is_dirty() && d.regen_from_memory);
}
