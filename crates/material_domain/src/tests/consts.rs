//! Global constant tables (decision 0017).

use super::*;

pub(super) fn tuning() -> AssetId {
    AssetId::new(AssetType::Consts, "common/tuning")
}

pub(super) fn global_tint() -> AssetId {
    AssetId::new(AssetType::Material, "fx/global_tint")
}

pub(super) fn set(e: &mut Editor, target: impl PropTarget, value: Value) {
    e.apply(set_value(target, value, false));
}

/// The OK action of the open confirmation dialog.
pub(super) fn confirm(e: &Editor) -> Action {
    match &e.dialog {
        Some(Dialog::Confirm { buttons, .. }) => buttons.last().unwrap().1.clone(),
        other => panic!("no confirmation: {other:?}"),
    }
}

pub(super) const GLOW_AMOUNT_ID: u32 = 1003;
pub(super) const FANCY_STRIPES_ID: u32 = 1006;

#[test]
fn consts_tab_adds_edits_and_saves() {
    let (dir, mut e) = project();
    e.apply(new_asset("consts", 0));
    e.apply(Action::Submit("more/extra".into()));
    let extra = AssetId::new(AssetType::Consts, "more/extra");
    assert_eq!(e.consts_index(&extra), e.active);
    assert!(e.doc().is_none(), "graph commands must not reach a table tab");
    assert!(e.consts_doc().unwrap().table.consts.is_empty());

    // The add form refuses an empty name, allows any non-empty string (including a name from another table), and
    // refuses duplicates in the same table.
    set(&mut e, ConstsProp::DraftName, Value::Str("   ".into()));
    e.apply(Action::from(ConstsAction::Add));
    let err = e.consts_doc().unwrap().form_error.clone().unwrap_or_default();
    assert!(err.contains("empty"), "{err}");

    set(&mut e, ConstsProp::DraftName, Value::Str("glow_amount".into()));
    set(&mut e, ConstsProp::DraftType, Value::Str("f32".into()));
    e.apply(Action::from(ConstsAction::Add));
    assert_eq!(e.consts_doc().unwrap().form_error, None, "names are unique per table, not project-wide");

    set(&mut e, ConstsProp::DraftName, Value::Str("glow_amount".into()));
    e.apply(Action::from(ConstsAction::Add));
    let err = e.consts_doc().unwrap().form_error.clone().unwrap_or_default();
    assert!(err.contains("already in this table"), "{err}");

    set(&mut e, ConstsProp::DraftName, Value::Str("wind".into()));
    set(&mut e, ConstsProp::DraftType, Value::Str("vec3".into()));
    e.apply(Action::from(ConstsAction::Add));
    let c = e.consts_doc().unwrap();
    assert_eq!((c.form_error.as_deref(), c.draft_name.as_str()), (None, ""));
    let wind = c.table.get("wind").unwrap();
    assert_eq!(wind.ty, ConstType::Vec3);
    let wind_id = wind.id;

    // A picker drag (all components, merged) is one undo step; a typed component another.
    let target = ConstsProp::Value(wind_id);
    for x in [0.2, 0.4] {
        for (comp, value) in [x, 0.5, 0.6, 1.0].into_iter().enumerate() {
            e.apply(set_component(target.clone(), comp, value, false));
        }
    }
    e.apply(Action::Tab(TabAction::CommitInline));
    e.apply(set_component(target.clone(), 2, 3.0, true));
    assert_eq!(shown(&e, &target), Some(Value::Float3([0.4, 0.5, 3.0])), "values beyond 0..1 are fine");
    e.apply(Action::Tab(TabAction::Undo));
    assert_eq!(shown(&e, &target), Some(Value::Float3([0.4, 0.5, 0.6])));
    e.apply(Action::Tab(TabAction::Undo));
    assert_eq!(shown(&e, &target), Some(Value::Float3([1.0; 3])), "new constants start at 1.0");
    e.apply(Action::Tab(TabAction::Redo));

    // Without readers a type change needs no confirmation (and converts the value).
    set(&mut e, ConstsProp::Type(wind_id), Value::Str("f32".into()));
    assert!(e.dialog.is_none());
    assert_eq!(shown(&e, &target), Some(Value::Float(0.4)));
    e.apply(Action::Save);
    let on_disk = assets::load_consts(e.project.as_ref().unwrap(), &extra).unwrap();
    assert_eq!(on_disk.get_by_id(wind_id).unwrap().value, Value::Float(0.4));

    // Editing a table regenerates the shaders reading it; unsaved values already show in the editor.
    e.apply(Action::OpenAsset(tuning().to_ref()));
    let amount = ConstsProp::Value(GLOW_AMOUNT_ID);
    e.apply(set_component(amount, 0, 0.9, true));
    assert_eq!(e.const_table("common/tuning").unwrap().get_by_id(GLOW_AMOUNT_ID).unwrap().value, Value::Float(0.9));
    e.apply(Action::Save);
    let wesl_path = dir.path().join("assets/quillshade/material/autogen/dir_fx/mat_global_tint.wesl");
    let wesl = std::fs::read_to_string(&wesl_path).unwrap();
    assert!(wesl.contains("0.9f"), "{wesl}");
    assert!(!dir.path().join("assets/quillshade/material/autogen/dir_common").exists());

    // Renaming a constant in the table (including non-identifier characters) keeps its id and does not change the
    // material RON or the generated WESL.
    let mat_path = dir.path().join("quillshade/material/fx/global_tint.material.ron");
    let mat_before = std::fs::read_to_string(&mat_path).unwrap();
    set(&mut e, ConstsProp::Name(GLOW_AMOUNT_ID), Value::Str("Glow Amount (HDR)!".into()));
    e.apply(Action::Save);
    let on_disk = assets::load_consts(e.project.as_ref().unwrap(), &tuning()).unwrap();
    assert_eq!(on_disk.get_by_id(GLOW_AMOUNT_ID).unwrap().name, "Glow Amount (HDR)!");
    assert_eq!(std::fs::read_to_string(&mat_path).unwrap(), mat_before);
    assert_eq!(std::fs::read_to_string(&wesl_path).unwrap(), wesl);
}

#[test]
fn breaking_const_edits_ask_and_clear_references() {
    let (_dir, mut e) = project();
    let usages = |e: &Editor, id: u32| e.const_usages("common/tuning", &[id]);
    e.apply(Action::OpenAsset(global_tint().to_ref()));
    e.apply(Action::OpenAsset(tuning().to_ref()));
    assert_eq!(usages(&e, GLOW_AMOUNT_ID), vec![(global_tint(), 1)]);

    // A type change of a constant in use waits for the confirmation.
    let ty = ConstsProp::Type(GLOW_AMOUNT_ID);
    set(&mut e, ty.clone(), Value::Str("vec2".into()));
    assert_eq!(shown(&e, &ty), Some(Value::Str("f32".into())), "not changed before OK");
    e.apply(Action::CloseDialog);
    set(&mut e, ty.clone(), Value::Str("vec2".into()));
    e.apply(confirm(&e));
    assert!(e.dialog.is_none());
    assert_eq!(shown(&e, &ty), Some(Value::Str("vec2".into())));
    // OK saves the table with the cleared references.
    assert!(!e.is_dirty());
    let table = assets::load_consts(e.project.as_ref().unwrap(), &tuning()).unwrap();
    assert_eq!(table.get_by_id(GLOW_AMOUNT_ID).unwrap().ty, ConstType::Vec2);
    // The open graph had no unsaved changes: edited and saved right away.
    let g = e.doc_index(&global_tint()).unwrap();
    let d = e.tabs[g].get::<Doc>().unwrap();
    assert!(!d.is_dirty());
    assert!(const_refs(&d.asset, &e.domains.material().registry).iter().all(|r| r.3 != GLOW_AMOUNT_ID));
    let on_disk = assets::load(e.project.as_ref().unwrap(), &global_tint()).unwrap();
    assert!(const_refs(&on_disk, &e.domains.material().registry).iter().all(|r| r.3 != GLOW_AMOUNT_ID));
    assert!(usages(&e, GLOW_AMOUNT_ID).is_empty());
    e.validate();
    let codes: Vec<Code> = e.domains.material().diags[&global_tint()].iter().map(|d| d.code).collect();
    assert_eq!(codes, vec![Code::W106]);

    // Undo / redo of a type change asks too once a node reads the constant again.
    let t = e.consts_index(&tuning()).unwrap();
    e.apply(Action::ActivateTab(g));
    let node = e.doc().unwrap().asset.nodes.iter().find(|(_, n)| n.kind == "stdlib::GlobalConstF32").unwrap();
    let param = GraphProp::Param { node: node.0.clone(), key: "const".into() };
    e.apply(Action::Tab(TabAction::Undo)); // the graph's clearing
    assert_eq!(shown(&e, &param), Some(Value::Int(GLOW_AMOUNT_ID as i32)));
    e.apply(Action::ActivateTab(t));
    e.apply(Action::Tab(TabAction::Undo));
    assert!(e.dialog.is_some(), "undoing the type change breaks the reader");
    assert_eq!(shown(&e, &ty), Some(Value::Str("vec2".into())));
    e.apply(confirm(&e));
    assert_eq!(shown(&e, &ty), Some(Value::Str("f32".into())));
    assert!(usages(&e, GLOW_AMOUNT_ID).is_empty());

    // Deleting a constant read by a closed graph rewrites that file.
    let g = e.doc_index(&global_tint()).unwrap();
    e.apply(Action::CloseTab(g));
    e.apply(Action::from(ConstsAction::Delete { id: FANCY_STRIPES_ID, confirmed: false }));
    assert!(e.consts_doc().unwrap().table.get_by_id(FANCY_STRIPES_ID).is_some());
    e.apply(confirm(&e));
    assert!(e.consts_doc().unwrap().table.get_by_id(FANCY_STRIPES_ID).is_none());
    let on_disk = assets::load(e.project.as_ref().unwrap(), &global_tint()).unwrap();
    assert!(const_refs(&on_disk, &e.domains.material().registry).iter().all(|r| r.3 != FANCY_STRIPES_ID));
    // An unused constant goes without asking (undo and delete again).
    e.apply(Action::Tab(TabAction::Undo));
    assert!(e.dialog.is_none());
    e.apply(Action::from(ConstsAction::Delete { id: FANCY_STRIPES_ID, confirmed: false }));
    assert!(e.dialog.is_none());
    assert!(e.consts_doc().unwrap().table.get_by_id(FANCY_STRIPES_ID).is_none());
}

#[test]
fn deleting_a_table_clears_the_references_to_its_constants() {
    let (_dir, mut e) = project();
    e.apply(Action::DeleteDialog(tuning().to_ref()));
    let Some(Dialog::Confirm { message, .. }) = &e.dialog else { panic!("no dialog") };
    assert!(message.contains("fx/global_tint.material.ron (6 nodes)"), "{message}");
    assert!(!message.contains("generated files"), "{message}");
    e.apply(confirm(&e));
    assert!(!tuning().file(e.project.as_ref().unwrap()).exists());
    let on_disk = assets::load(e.project.as_ref().unwrap(), &global_tint()).unwrap();
    assert!(const_refs(&on_disk, &e.domains.material().registry).is_empty());
    e.validate();
    assert!(e.domains.material().diags[&global_tint()].iter().all(|d| d.code == Code::W106));
}

#[test]
fn consts_tabs_are_restored_and_follow_renames() {
    let (dir, mut e) = project();
    e.apply(Action::OpenAsset(tuning().to_ref()));
    e.rename(&tuning().to_ref(), "settings/tuning");
    let moved = AssetId::new(AssetType::Consts, "settings/tuning");
    assert_eq!(e.consts_doc().map(|c| &c.id), Some(&moved));
    assert!(
        e.domains.material().diags.values().flatten().all(|d| !d.is_error()),
        "table references are rewritten: users are unaffected"
    );
    e.save_tabs_to_config();
    let config = e.config.clone();
    let mut next = crate::test_editor().with_config(config, None);
    next.open_project(dir.path()).unwrap();
    assert_eq!(next.consts_doc().map(|c| &c.id), Some(&moved));
}

#[test]
fn boolean_const_types_and_compile_bool_pruning() {
    let (dir, mut e) = project();
    e.apply(Action::OpenAsset(tuning().to_ref()));
    for (name, label, ty) in [
        ("my_toggle", "toggle", ConstType::Toggle),
        ("my_runtime", "runtimeBool", ConstType::RuntimeBool),
        ("my_compile", "compileBool", ConstType::CompileBool),
    ] {
        set(&mut e, ConstsProp::DraftName, Value::Str(name.into()));
        set(&mut e, ConstsProp::DraftType, Value::Str(label.into()));
        e.apply(Action::from(ConstsAction::Add));
        let c = e.consts_doc().unwrap().table.get(name).unwrap().clone();
        assert_eq!((c.ty, &c.value), (ty, &Value::Bool(true)));
        let val = ConstsProp::Value(c.id);
        set(&mut e, val.clone(), Value::Bool(false));
        assert_eq!(shown(&e, &val), Some(Value::Bool(false)));
    }

    let stripes = ConstsProp::Value(FANCY_STRIPES_ID);
    assert_eq!(shown(&e, &stripes), Some(Value::Bool(true)));
    set(&mut e, stripes.clone(), Value::Bool(false));
    e.apply(Action::Save);
    let wesl_path = dir.path().join("assets/quillshade/material/autogen/dir_fx/mat_global_tint.wesl");
    let wesl = std::fs::read_to_string(&wesl_path).unwrap();
    assert!(!wesl.contains("fract("), "inactive branch pruned when compileBool is false:\n{wesl}");

    set(&mut e, stripes, Value::Bool(true));
    e.apply(Action::Save);
    let wesl = std::fs::read_to_string(&wesl_path).unwrap();
    assert!(wesl.contains("fract("), "active branch emitted when compileBool is true:\n{wesl}");
}

#[test]
fn const_node_props_reset_const_on_table_change() {
    let (_dir, mut e) = project();
    let other = ops::create_consts(e.project.as_ref().unwrap(), "common/other").unwrap();
    e.rescan();
    e.apply(Action::OpenAsset(global_tint().to_ref()));
    let node = e.doc().unwrap().asset.nodes.iter().find(|(_, n)| n.kind == "stdlib::GlobalConstF32").unwrap().0.clone();
    let table_prop = GraphProp::Param { node: node.clone(), key: "table".into() };
    let const_prop = GraphProp::Param { node: node.clone(), key: "const".into() };
    assert_eq!(shown(&e, &table_prop), Some(Value::Str("common/tuning".into())));
    assert_eq!(shown(&e, &const_prop), Some(Value::Int(GLOW_AMOUNT_ID as i32)));
    assert_eq!(e.referenced_asset(&node), Some(tuning()));

    // Picking another table resets the constant id to 0 in the same undo step.
    set(&mut e, table_prop.clone(), Value::Str("common/other".into()));
    assert_eq!(shown(&e, &table_prop), Some(Value::Str("common/other".into())));
    assert_eq!(shown(&e, &const_prop), Some(Value::Int(0)));
    assert_eq!(e.referenced_asset(&node), Some(other));
    e.apply(Action::Tab(TabAction::Undo));
    assert_eq!(shown(&e, &table_prop), Some(Value::Str("common/tuning".into())));
    assert_eq!(shown(&e, &const_prop), Some(Value::Int(GLOW_AMOUNT_ID as i32)));

    // `keyed_select` sends a string of the constant id (or empty for `(none)`), which `model_value_for` parses to `Int`.
    e.apply(set_value(const_prop.clone(), Value::Str(String::new()), false));
    assert_eq!(shown(&e, &const_prop), Some(Value::Int(0)));
    e.apply(set_value(const_prop.clone(), Value::Str(GLOW_AMOUNT_ID.to_string()), false));
    assert_eq!(shown(&e, &const_prop), Some(Value::Int(GLOW_AMOUNT_ID as i32)));
}
