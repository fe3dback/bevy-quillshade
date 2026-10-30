//! Gradient collections (decision 0037).

use super::*;

use shader_graph_core::gradient::{SLOTS, Slot};

/// The test project with a collection `common/ramps` on disk, opened in a tab.
pub(super) fn gradients_project() -> (tempfile::TempDir, Editor, AssetId) {
    let (dir, mut e) = project();
    let id = ops::create_gradients(e.project.as_ref().unwrap(), "common/ramps").unwrap();
    e.rescan();
    e.apply(Action::OpenAsset(id.to_ref()));
    (dir, e, id)
}

pub(super) fn gdoc(e: &Editor) -> &GradientsDoc {
    e.gradients_doc().expect("an active gradients tab")
}

pub(super) fn stops(e: &Editor) -> Vec<Stop> {
    let d = gdoc(e);
    d.collection.slots[d.slot].stops.clone()
}

pub(super) fn ga(e: &mut Editor, a: GradientsAction) {
    e.apply(Action::from(a));
}

#[test]
fn gradients_create_open_save_reload() {
    let (dir, mut e) = project();
    e.apply(new_asset("gradients", 0));
    let Some(Dialog::Input { initial, title, .. }) = &e.dialog else { panic!("no dialog") };
    assert_eq!(initial, "new_gradients");
    assert_eq!(title, "New Gradient Collection");
    e.apply(Action::Submit("new_gradients".into()));
    assert!(dir.path().join("quillshade/material/new_gradients.gradients.ron").is_file());
    let id = AssetId::new(AssetType::Gradients, "new_gradients");
    assert_eq!(e.asset_index(&id), e.active);
    assert_eq!(e.tab().unwrap().doc().view(), crate::editors::gradients::GRADIENTS);
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
    e.apply(Action::Reload(asset_key(&id)));
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
    e.apply(Action::Tab(TabAction::CommitInline));
    e.apply(Action::Tab(TabAction::Undo));
    assert_eq!(stops(&e), before);

    // Dragging a stop past its neighbour re-sorts; the selection follows it.
    ga(&mut e, GradientsAction::SetStopColor(0, [1.0, 0.0, 0.0, 1.0], None));
    ga(&mut e, GradientsAction::SetStopT(0, 0.7, Some("drag".into())));
    assert_eq!(gdoc(&e).selection, Some(GradientSel::Stop(1)));
    assert_eq!(stops(&e)[1].color, [1.0, 0.0, 0.0, 1.0]);
    assert_eq!(stops(&e)[1].t, 0.7);
    e.apply(Action::Tab(TabAction::CommitInline));
    e.apply(Action::Tab(TabAction::Undo));
    e.apply(Action::Tab(TabAction::Undo));
    assert_eq!(stops(&e), before);

    // Midpoints are clamped and belong to their stop.
    ga(&mut e, GradientsAction::SetMid(0, 5.0, None));
    assert_eq!(stops(&e)[0].mid, shader_graph_core::gradient::MID_MAX);
    e.apply(Action::Tab(TabAction::Undo));

    ga(&mut e, GradientsAction::Select(Some(GradientSel::Stop(1))));
    e.apply(Action::Tab(TabAction::DeleteSelection));
    assert_eq!(stops(&e).len(), 2);
    assert_eq!(gdoc(&e).selection, None);
    e.apply(Action::Tab(TabAction::Undo));
    assert_eq!(stops(&e), before);
    e.apply(Action::Tab(TabAction::Redo));
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
    e.apply(Action::Tab(TabAction::CommitInline));
    ga(&mut e, GradientsAction::SetStopColor(0, [0.0, 0.0, 1.0, 1.0], Some("color".into())));
    e.apply(Action::Tab(TabAction::CommitInline));
    e.apply(Action::Tab(TabAction::Undo));
    assert_eq!(stops(&e)[0].color, [0.6, 0.2, 0.1, 0.5], "the second pick is its own step");
    e.apply(Action::Tab(TabAction::Undo));
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
        let (target, value) = (GradientProp::Name, Value::Str(name.into()));
        e.apply(set_value(target, value, true));
    }
    e.apply(Action::Tab(TabAction::Undo));
    assert_eq!(gdoc(&e).collection.slots[0].name, "");
    e.apply(Action::Tab(TabAction::Redo));
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
    e.apply(Action::Tab(TabAction::Copy));
    ga(&mut e, GradientsAction::SelectSlot(7));
    assert_eq!(gdoc(&e).slot, 7);
    e.apply(Action::Tab(TabAction::Paste(None)));
    let s7 = gdoc(&e).collection.slots[7].clone();
    assert_eq!((s7.id, &s7.name, &s7.stops), (ids[7], &slot0.name, &slot0.stops));

    // Duplicate goes to the next empty slot after the selected one.
    e.apply(Action::Tab(TabAction::Duplicate));
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
    e.apply(Action::Tab(TabAction::Undo));
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
    assert_eq!(key, "quillshade/material/common/ramps.gradients.ron");
    assert_eq!(parse_config_key(&key), Some(MaterialKey::Asset(id.clone())));
    e.save_tabs_to_config();
    let mut next = crate::test_editor().with_config(e.config.clone(), None);
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
    let set =
        |key: &str, value: Value| set_value(GraphProp::Param { node: node.clone(), key: key.into() }, value, false);
    e.apply(set(GRADIENT_COLLECTION_PARAM, Value::Str("common/ramps".into())));
    e.apply(set(GRADIENT_SLOT_PARAM, Value::Int(slot_id)));
    e.apply(Action::Save);
    let doc = |e: &Editor| e.tabs[e.doc_index(&rock).unwrap()].get::<Doc>().unwrap().clone();
    let refs = |e: &Editor| {
        gradient_refs(&doc(e).asset, &e.domains.material().registry).into_iter().map(|r| r.2).collect::<Vec<_>>()
    };
    let params = |e: &Editor| {
        let n = &doc(e).asset.nodes[&node];
        (n.params[GRADIENT_COLLECTION_PARAM].clone(), n.params[GRADIENT_SLOT_PARAM].clone())
    };

    // Clean: the rewritten file is reloaded.
    e.apply(Action::RenameDialog(id.to_ref()));
    e.apply(Action::Submit("env/terrain".into()));
    let moved = AssetId::new(AssetType::Gradients, "env/terrain");
    assert!(e.asset_index(&moved).is_some(), "the gradients tab follows");
    assert_eq!(refs(&e), vec!["env/terrain"]);
    assert!(!doc(&e).is_dirty());

    // Dirty: the open material gets the new id as an unsaved edit, the slot is kept.
    let i = e.doc_index(&rock).unwrap();
    e.tabs[i].get_mut::<Doc>().unwrap().edit(|a| a.nodes.get_mut(&node).unwrap().pos.0 += 10.0);
    e.apply(Action::RenameDialog(moved.to_ref()));
    e.apply(Action::Submit("env/ground".into()));
    let moved = AssetId::new(AssetType::Gradients, "env/ground");
    assert!(doc(&e).is_dirty(), "unsaved edits stay unsaved");
    assert_eq!(params(&e), (Value::Str("env/ground".into()), Value::Int(slot_id)));

    e.apply(Action::Delete(moved.to_ref()));
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
    let set =
        |key: &str, value: Value| set_value(GraphProp::Param { node: node.clone(), key: key.into() }, value, false);
    let params = |e: &Editor| (param_of(e, &node, GRADIENT_COLLECTION_PARAM), param_of(e, &node, GRADIENT_SLOT_PARAM));
    e.apply(set(GRADIENT_COLLECTION_PARAM, Value::Str("common/ramps".into())));
    e.apply(set(GRADIENT_SLOT_PARAM, Value::Int(slot_id)));
    assert_eq!(e.referenced_asset(&node), Some(id));
    let before = params(&e);
    assert_eq!(before, (Some(Value::Str("common/ramps".into())), Some(Value::Int(slot_id))));

    e.apply(set(GRADIENT_COLLECTION_PARAM, Value::Str("common/sky".into())));
    assert_eq!(params(&e), (Some(Value::Str("common/sky".into())), Some(Value::Int(0))));
    assert_eq!(e.referenced_asset(&node), Some(sky));
    e.apply(Action::Tab(TabAction::Undo));
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
    use crate::ops::droppable::Dropped;
    use shader_stdlib::nodes::color::{GRADIENT_COLLECTION_PARAM, GRADIENT_SLOT_PARAM, SAMPLE_GRADIENT_KIND};
    let (_dir, mut e, _id) = gradients_project();
    let slot_id = gdoc(&e).collection.slots[3].id as i32;
    ops::create_gradients(e.project.as_ref().unwrap(), "common/sky").unwrap();
    e.rescan();
    open_rock_wet(&mut e);
    let node = add(&mut e, SAMPLE_GRADIENT_KIND);
    let set =
        |key: &str, value: Value| set_value(GraphProp::Param { node: node.clone(), key: key.into() }, value, false);
    let params = |e: &Editor| (param_of(e, &node, GRADIENT_COLLECTION_PARAM), param_of(e, &node, GRADIENT_SLOT_PARAM));
    e.apply(set(GRADIENT_COLLECTION_PARAM, Value::Str("common/ramps".into())));
    e.apply(set(GRADIENT_SLOT_PARAM, Value::Int(slot_id)));
    let picked = (Some(Value::Str("common/ramps".into())), Some(Value::Int(slot_id)));
    assert_eq!(params(&e), picked);

    e.apply(set(GRADIENT_COLLECTION_PARAM, Value::Str("common/ramps".into())));
    assert_eq!(params(&e), picked, "setting the same collection keeps the slot");
    let item = Dropped::Gradients("common/ramps".into());
    e.apply(MaterialAction::Drop { item, pos: (0.0, 0.0), node: Some(node.clone()) });
    assert_eq!(params(&e), picked, "dropping the node's own collection keeps the slot");

    let item = Dropped::Gradients("common/sky".into());
    e.apply(MaterialAction::Drop { item, pos: (0.0, 0.0), node: Some(node.clone()) });
    assert_eq!(params(&e), (Some(Value::Str("common/sky".into())), Some(Value::Int(0))));
    e.apply(Action::Tab(TabAction::Undo));
    assert_eq!(params(&e), picked, "one undo step restores both");
}

/// Gradient design §6: a collection dropped onto a graph adds a Sample Gradient reading it, no gradient picked; onto a
/// Sample Gradient it replaces the collection and resets the slot.
#[test]
fn gradient_drop_creates_sample_gradient() {
    use crate::ops::droppable::Dropped;
    use crate::ui::tree::{TreeItem, TreeRow};
    use shader_stdlib::nodes::color::{GRADIENT_COLLECTION_PARAM, GRADIENT_SLOT_PARAM, SAMPLE_GRADIENT_KIND};
    let (_dir, mut e, id) = gradients_project();
    let slot_id = gdoc(&e).collection.slots[3].id as i32;
    let row = TreeRow { tree: TreeId::Project(MATERIAL), item: TreeItem::Asset(id.to_ref()) };
    assert_eq!(Dropped::from_row(&row), Some(Dropped::Gradients("common/ramps".into())));

    open_rock_wet(&mut e);
    let item = Dropped::Gradients("common/ramps".into());
    e.apply(MaterialAction::Drop { item, pos: (21.0, 9.0), node: None });
    let added = nodes_of(&e, SAMPLE_GRADIENT_KIND);
    assert_eq!(added.len(), 1);
    let node = added[0].clone();
    assert_eq!(e.doc().unwrap().asset.nodes[&node].pos, (20.0, 10.0));
    assert_eq!(param_of(&e, &node, GRADIENT_COLLECTION_PARAM), Some(Value::Str("common/ramps".into())));
    let slot = e
        .domains
        .material()
        .registry
        .resolved_params(SAMPLE_GRADIENT_KIND, &e.doc().unwrap().asset.nodes[&node].params);
    assert_eq!(slot.get(GRADIENT_SLOT_PARAM), Some(&Value::Int(0)), "no gradient picked");
    assert_eq!(e.doc().unwrap().selection, [node.clone()].into());
    assert_eq!(e.referenced_asset(&node), Some(id));

    // Onto the node: the collection is replaced and the slot reset.
    ops::create_gradients(e.project.as_ref().unwrap(), "common/sky").unwrap();
    e.rescan();
    let target = GraphProp::Param { node: node.clone(), key: GRADIENT_SLOT_PARAM.into() };
    e.apply(set_value(target, Value::Int(slot_id), false));
    let count = e.doc().unwrap().asset.nodes.len();
    let item = Dropped::Gradients("common/sky".into());
    e.apply(MaterialAction::Drop { item, pos: (0.0, 0.0), node: Some(node.clone()) });
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
    let set =
        |key: &str, value: Value| set_value(GraphProp::Param { node: node.clone(), key: key.into() }, value, false);
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
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet").to_ref()));
    let decl = add(&mut e, "stdlib::NamedRoute");
    let kinds = |e: &Editor, kind: &str| e.doc().unwrap().asset.nodes.values().filter(|n| n.kind == kind).count();
    let selected_route = |e: &Editor| {
        let d = e.doc().unwrap();
        assert_eq!(d.selection.len(), 1);
        let n = &d.asset.nodes[d.selection.first().unwrap()];
        assert_eq!(n.kind, "stdlib::NamedRouteUsage");
        n.params.get(shader_graph_core::registry::ROUTE_PARAM).cloned()
    };
    e.apply(GraphEdit::Select([decl.clone()].into()));
    e.apply(Action::Tab(TabAction::Duplicate));
    assert_eq!(selected_route(&e), Some(Value::Str(decl.clone())));
    assert_eq!(kinds(&e, "stdlib::NamedRoute"), 1);
    e.apply(Action::Tab(TabAction::Undo));
    assert_eq!(kinds(&e, "stdlib::NamedRouteUsage"), 0, "one undo step");

    e.apply(GraphEdit::Select([decl.clone()].into()));
    e.apply(Action::Tab(TabAction::Copy));
    e.apply(Action::Tab(TabAction::Paste(None)));
    assert_eq!(selected_route(&e), Some(Value::Str(decl.clone())));
    assert_eq!(kinds(&e, "stdlib::NamedRoute"), 1);

    // Another graph tab: the declaration is duplicated.
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "fx/hologram").to_ref()));
    assert_eq!(active_asset(&e), AssetId::new(AssetType::Material, "fx/hologram"));
    let before = kinds(&e, "stdlib::NamedRoute");
    e.apply(Action::Tab(TabAction::Paste(None)));
    assert_eq!(kinds(&e, "stdlib::NamedRoute"), before + 1);
    assert_eq!(kinds(&e, "stdlib::NamedRouteUsage"), 0);
}

#[test]
fn opening_sample_gradient_node_selects_configured_slot() {
    use shader_stdlib::nodes::color::{GRADIENT_COLLECTION_PARAM, GRADIENT_SLOT_PARAM, SAMPLE_GRADIENT_KIND};

    let (_dir, mut e, id) = gradients_project();
    let slot_id = gdoc(&e).collection.slots[5].id;
    // Select slot 0 in the gradients tab first so we verify opening the node switches to slot 5.
    ga(&mut e, GradientsAction::SelectSlot(0));
    assert_eq!(gdoc(&e).slot, 0);

    open_rock_wet(&mut e);
    let node = add(&mut e, SAMPLE_GRADIENT_KIND);
    let set =
        |key: &str, value: Value| set_value(GraphProp::Param { node: node.clone(), key: key.into() }, value, false);
    e.apply(set(GRADIENT_COLLECTION_PARAM, Value::Str("common/ramps".into())));

    // With slot 0 (no slot picked), open_action falls back to plain Action::OpenAsset.
    assert_eq!(e.referenced_gradient_slot(&node), None);
    assert_eq!(e.open_action(&node), Some(Action::OpenAsset(id.to_ref())));

    // With a valid slot picked, open_action (and header_action) returns MaterialAction::OpenGradient.
    e.apply(set(GRADIENT_SLOT_PARAM, Value::Int(slot_id as i32)));
    assert_eq!(e.referenced_gradient_slot(&node), Some(slot_id));
    let expected: Action = MaterialAction::OpenGradient { id: id.clone(), slot_id }.into();
    assert_eq!(e.open_action(&node), Some(expected.clone()));
    assert_eq!(e.header_action(&node), Some(expected.clone()));

    e.apply(expected);
    assert_eq!(gdoc(&e).id, id);
    assert_eq!(gdoc(&e).slot, 5);
}
