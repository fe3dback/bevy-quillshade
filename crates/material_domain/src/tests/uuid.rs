//! The resolved view (ADR 0047 §4, read side): a project whose sources store references as UUIDs
//! ([`fixtures::save_all`]) shows exactly what the same project with path references shows
//! ([`fixtures::save_all_path_form`]); and the write side: everything that creates a reference stores the UUID.

use super::*;
use crate::ui::const_preview::ConstPreview;
use crate::ui::curve_preview::CurvePreview;
use crate::ui::gradient_preview::GradientPreview;
use crate::ui::node_preview::NodePreview;
use crate::ui::texture_preview::TexturePreview;
use shader_project::MaterialLayout;

/// Copies the directory tree `from` into `to` (hidden folders such as `.nodes` included).
fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// An editor on a copy of the project in `dir` with references stored as paths, converted to the UUID form.
fn uuid_copy(dir: &Path) -> (tempfile::TempDir, Editor) {
    let copy = tempfile::tempdir().unwrap();
    copy_dir(dir, copy.path());
    fixtures::convert_to_uuid_form(&Project::open(copy.path()).unwrap()).unwrap();
    let mut e = crate::test_editor();
    e.open_project(copy.path()).unwrap();
    (copy, e)
}

/// The sample fixtures in the UUID form.
fn uuid_project() -> (tempfile::TempDir, Editor) {
    project()
}

fn graph_id(path: &str, is_fn: bool) -> AssetId {
    AssetId::new(if is_fn { AssetType::Function } else { AssetType::Material }, path)
}

/// The node of the active graph whose resolved kind is `kind`.
fn node_of_kind(e: &Editor, kind: &str) -> NodeId {
    let view = e.view_of(e.doc().unwrap());
    view.nodes.iter().find(|(_, n)| n.kind == kind).map(|(id, _)| id.clone()).unwrap_or_else(|| panic!("no {kind}"))
}

/// What the graph editor shows of each node of graph `id`, opened and validated: kind (the right sidebar's subtitle),
/// title, pin signature, property values (params and input defaults), the referenced asset and the actions opening
/// it, the node previews, the preview refusal and the node's problems.
fn shown_graph(e: &mut Editor, id: &AssetId) -> Vec<String> {
    e.apply(Action::OpenAsset(id.to_ref()));
    e.validate();
    let d = e.doc().unwrap();
    assert_eq!(d.id, *id);
    let view = e.view_of(d);
    let (registry, functions) = (&e.domains.material().registry, &e.domains.material().functions);
    let diags = e.node_diags();
    let mut out = Vec::new();
    for (node, n) in &view.nodes {
        let mut line = vec![format!("{node} {}", n.kind), crate::doc::node_title(registry, functions, &view, node)];
        let sig = node_signature(&view, node, registry, functions, &d.id.path);
        line.push(format!("{sig:?}"));
        for p in registry.get(&n.kind).map(|def| def.params()).unwrap_or_default() {
            let key = GraphProp::Param { node: node.clone(), key: p.id.clone() };
            line.push(format!("{} = {:?}", p.id, shown(e, &key)));
        }
        for pin in sig.iter().flat_map(|s| &s.inputs) {
            let key = GraphProp::Input { node: node.clone(), pin: pin.id.clone() };
            line.push(format!("{} = {:?}", pin.id, shown(e, &key)));
        }
        line.push(format!("{:?}", e.referenced_asset(node)));
        line.push(format!("{:?} {:?}", e.open_action(node), e.header_action(node)));
        line.push(format!("{:?}", e.preview_refusal(node)));
        let curve = CurvePreview::key(e, d, node).map(|k| (k.curve.name, k.curve.points, k.interp, k.t));
        line.push(format!("{curve:?}"));
        line.push(format!("{:?}", ConstPreview::key(e, d, node)));
        line.push(format!("{:?}", GradientPreview::key(e, d, node)));
        line.push(format!("{:?}", TexturePreview::key(e, d, node).map(|k| (k.image, k.settings))));
        line.push(format!("{:?}", diags.get(node)));
        out.push(line.join(" | "));
    }
    out
}

#[test]
fn uuid_sources_show_like_path_sources() {
    let (paths_dir, mut paths) = path_form_project();
    let (_uuids_dir, mut uuids) = uuid_copy(paths_dir.path());
    let mut stored_as_uuid = Vec::new();
    for (path, is_fn, _) in fixtures::all() {
        let id = graph_id(path, is_fn);
        assert_eq!(shown_graph(&mut uuids, &id), shown_graph(&mut paths, &id), "{path}");
        let d = paths.doc().unwrap();
        assert_eq!(d.asset.nodes, paths.view_of(d).nodes, "{path}: a path-form view is the model");
        let d = uuids.doc().unwrap();
        if d.asset.nodes != uuids.view_of(d).nodes {
            stored_as_uuid.push(path);
        }
    }
    // Every reference kind is covered: textures, functions, curves, constant tables, gradient collections, user nodes.
    for path in
        ["env/rock_wet", "env/tex_layers", "fx/curve_bands", "fx/global_tint", "env/terrain_layers", "fx/user_node"]
    {
        assert!(stored_as_uuid.contains(&path), "{path} stores no UUID: {stored_as_uuid:?}");
    }
    // The project's diagnostics too (validation resolves through the editor's index).
    assert_eq!(uuids.domains.material().diags, paths.domains.material().diags);
}

#[test]
fn opening_the_referenced_asset_follows_the_uuid() {
    let (_dir, mut e) = uuid_project();
    // A function call: the node stores `function::<uuid>`, the canvas and `SHADER_EDITOR_SELECT` see the path.
    e.apply(Action::OpenAsset(graph_id("env/tex_layers", false).to_ref()));
    let call = node_of_kind(&e, "function::common/tex_layer");
    let stored = &e.doc().unwrap().asset.nodes[&call].kind;
    assert!(project_core::AssetUuid::parse(stored.strip_prefix("function::").unwrap()).is_some(), "{stored}");
    e.apply(e.header_action(&call).unwrap());
    assert_eq!(e.doc().unwrap().id, graph_id("common/tex_layer", true));

    // A curve.
    e.apply(Action::OpenAsset(graph_id("fx/curve_bands", false).to_ref()));
    let sample = node_of_kind(&e, "stdlib::SampleCurve");
    let curve = param_of(&e, &sample, "curve").unwrap();
    e.apply(e.open_action(&sample).unwrap());
    let open = e.tab().and_then(|t| t.doc().asset().cloned());
    assert_eq!(open, Some(AssetId::new(AssetType::Curve, curve.as_str()).to_ref()));

    // A gradient collection, with the node's slot selected.
    e.apply(Action::OpenAsset(graph_id("env/terrain_layers", false).to_ref()));
    let sample = node_of_kind(&e, "stdlib::SampleGradient");
    let slot = e.referenced_gradient_slot(&sample).expect("the slot resolves through the collection's UUID");
    e.apply(e.open_action(&sample).unwrap());
    let g = e.gradients_doc().unwrap();
    assert_eq!(g.id, AssetId::new(AssetType::Gradients, "common/terrain"));
    assert_eq!(g.collection.position(slot), Some(g.slot));

    // A constant table and a Texture asset.
    e.apply(Action::OpenAsset(global_tint().to_ref()));
    let global = node_of_kind(&e, "stdlib::GlobalConstF32");
    e.apply(e.open_action(&global).unwrap());
    assert_eq!(e.consts_doc().unwrap().id, tuning());
    e.apply(Action::OpenAsset(graph_id("env/rock_wet", false).to_ref()));
    let albedo = e.view_of(e.doc().unwrap()).nodes.iter().find_map(|(id, n)| {
        (n.params.get("default") == Some(&Value::Str("textures/rock_albedo".into()))).then(|| id.clone())
    });
    let albedo = albedo.expect("a parameter with the albedo texture");
    assert_eq!(e.referenced_asset(&albedo), Some(AssetId::new(AssetType::Texture, "textures/rock_albedo")));
}

#[test]
fn const_usages_and_clearing_match_uuid_references() {
    let (_dir, mut e) = uuid_project();
    e.apply(Action::OpenAsset(global_tint().to_ref()));
    e.apply(Action::OpenAsset(tuning().to_ref()));
    assert_eq!(e.const_usages("common/tuning", &[GLOW_AMOUNT_ID]), vec![(global_tint(), 1)]);
    // A confirmed type change clears the UUID-form references of the open (saved) graph and of the closed ones.
    set(&mut e, ConstsProp::Type(GLOW_AMOUNT_ID), Value::Str("vec2".into()));
    e.apply(confirm(&e));
    assert!(e.const_usages("common/tuning", &[GLOW_AMOUNT_ID]).is_empty());
    let on_disk = assets::load(e.project.as_ref().unwrap(), &global_tint()).unwrap();
    assert!(const_refs(&on_disk, &e.domains.material().registry).iter().all(|r| r.3 != GLOW_AMOUNT_ID));
    assert!(!const_refs(&on_disk, &e.domains.material().registry).is_empty(), "other constants stay");
}

// ---- write side (ADR 0047 §4: everything that creates a reference stores the target's UUID) ----

/// The UUID the fixtures give the Material asset `ty` at `path`.
fn uuid_of(ty: AssetType, path: &str) -> String {
    fixtures::fixture_ref(ty, path)
}

/// The UUID the fixtures give the user node file `.nodes/<name>.ron`.
fn user_uuid(name: &str) -> String {
    fixtures::fixture_user_id(name).to_string()
}

/// The node `act` adds to the active graph.
fn added_by(e: &mut Editor, act: impl Into<Action>) -> NodeId {
    let before: BTreeSet<NodeId> = e.doc().unwrap().asset.nodes.keys().cloned().collect();
    e.apply(act);
    let new: Vec<NodeId> = e.doc().unwrap().asset.nodes.keys().filter(|k| !before.contains(*k)).cloned().collect();
    assert_eq!(new.len(), 1, "{new:?}");
    new[0].clone()
}

fn dropped(item: Dropped, node: Option<&NodeId>) -> MaterialAction {
    MaterialAction::Drop { item, pos: (0.0, 600.0), node: node.cloned() }
}

/// The stored kind and the kind of the resolved view of `node`.
fn kinds(e: &Editor, node: &str) -> (String, String) {
    let d = e.doc().unwrap();
    (d.asset.nodes[node].kind.clone(), e.view_of(d).nodes[node].kind.clone())
}

fn stored_param(e: &Editor, node: &str, key: &str) -> Option<Value> {
    e.doc().unwrap().asset.nodes[node].params.get(key).cloned()
}

use crate::ops::droppable::Dropped;
use shader_stdlib::internal::DEFAULT_TEXTURE;
use shader_stdlib::nodes::color::GRADIENT_COLLECTION_PARAM;

/// Drops of curves, Texture assets (by row or by game image), gradient collections and functions store the target's
/// UUID; the resolved view shows its path. Built-in textures stay as they are.
#[test]
fn drops_store_the_target_uuid() {
    let (dir, mut e) = uuid_project();
    open_rock_wet(&mut e);
    for (item, ty, path, key) in [
        (Dropped::Curve("common/pulse".into()), AssetType::Curve, "common/pulse", "curve"),
        (Dropped::Texture("textures/rock_albedo".into()), AssetType::Texture, "textures/rock_albedo", "texture"),
        (
            Dropped::Gradients("common/terrain".into()),
            AssetType::Gradients,
            "common/terrain",
            GRADIENT_COLLECTION_PARAM,
        ),
    ] {
        let node = added_by(&mut e, dropped(item, None));
        assert_eq!(stored_param(&e, &node, key), Some(Value::Str(uuid_of(ty, path))), "{path}");
        assert_eq!(param_of(&e, &node, key), Some(Value::Str(path.into())), "{path}");
        assert_eq!(e.referenced_asset(&node), Some(AssetId::new(ty, path)));
    }
    let call = added_by(&mut e, dropped(Dropped::Function("common/tex_layer".into()), None));
    let stored = format!("function::{}", uuid_of(AssetType::Function, "common/tex_layer"));
    assert_eq!(kinds(&e, &call), (stored, "function::common/tex_layer".into()));
    let internal = added_by(&mut e, dropped(Dropped::Texture(DEFAULT_TEXTURE.into()), None));
    assert_eq!(stored_param(&e, &internal, "texture"), Some(Value::Str(DEFAULT_TEXTURE.into())));

    // A game image whose Texture asset exists: that asset's UUID.
    game_file(&dir, &mut e, "textures/rock_albedo.ktx2");
    let before: BTreeSet<NodeId> = e.doc().unwrap().asset.nodes.keys().cloned().collect();
    drop_file(&mut e, "textures/rock_albedo.ktx2", (0.0, 900.0), None);
    let image = e.doc().unwrap().asset.nodes.keys().find(|k| !before.contains(*k)).cloned().unwrap();
    let rock = uuid_of(AssetType::Texture, "textures/rock_albedo");
    assert_eq!(stored_param(&e, &image, "texture"), Some(Value::Str(rock)));

    // Onto a node with a param of that kind: the param gets the UUID, in one undo step.
    let sample = node_of_kind(&e, "stdlib::SampleCurve");
    e.apply(dropped(Dropped::Curve("common/steps".into()), Some(&sample)));
    assert_eq!(stored_param(&e, &sample, "curve"), Some(Value::Str(uuid_of(AssetType::Curve, "common/steps"))));
    assert_eq!(param_of(&e, &sample, "curve"), Some(Value::Str("common/steps".into())));
    e.apply(Action::Tab(TabAction::Undo));
    assert_eq!(param_of(&e, &sample, "curve"), Some(Value::Str("common/pulse".into())));
}

/// A target without an id (a hand-written file, AE004) cannot be referenced (graphs store UUIDs only): a drop adds
/// nothing and says why, the palette lists the function disabled, the picker does not offer it.
#[test]
fn targets_without_an_id_are_not_referenced() {
    let (dir, mut e) = project();
    let material = dir.path().join("quillshade/material");
    let curve = material.join("common/bare.curve.ron");
    std::fs::write(&curve, shader_graph_core::Curve::ramp("bare").to_ron()).unwrap();
    let function = material.join("common/bare_fn.function.ron");
    let template = shader_project::ops::template("bare_fn", shader_graph_core::model::AssetKind::Function);
    std::fs::write(&function, template.to_ron()).unwrap();
    e.files_changed(vec![curve, function]);
    assert!(e.curve_paths().contains(&"common/bare".to_string()), "the curve loads");
    assert!(e.console_diagnostics().iter().filter(|d| d.code == "AE004").count() == 2);
    open_rock_wet(&mut e);
    let before = e.doc().unwrap().asset.clone();
    let warnings = e.messages.iter().filter(|m| m.level == Level::Warning).count();

    e.apply(dropped(Dropped::Curve("common/bare".into()), None));
    e.apply(dropped(Dropped::Function("common/bare_fn".into()), None));
    assert_eq!(e.doc().unwrap().asset, before, "nothing added");
    let new: Vec<&Message> = e.messages.iter().filter(|m| m.level == Level::Warning).skip(warnings).collect();
    assert_eq!(new.len(), 2, "{new:?}");
    assert!(new.iter().all(|m| m.text.contains(crate::graph_domain::NEEDS_UUID)), "{new:?}");
    let entries = super::palette::palette_entries(&e, None);
    let entry = entries.iter().find(|x| x.id == "function::common/bare_fn").expect("listed");
    assert_eq!(entry.disabled.as_deref(), Some(crate::graph_domain::NEEDS_UUID));
    let (choices, current) = crate::editors::graph::reference_choices(&e, AssetType::Curve, "common/pulse");
    assert_eq!(current, uuid_of(AssetType::Curve, "common/pulse"));
    assert!(choices.iter().all(|c| c.1 != "common/bare"), "{choices:?}");
    assert!(choices.contains(&(uuid_of(AssetType::Curve, "common/steps"), "common/steps".into())), "{choices:?}");
}

/// The palette inserts function calls and user nodes by UUID; a user node in the UUID format without an id is
/// listed disabled ("needs a UUID (AE004)") until Assign UUID gives it one.
#[test]
fn palette_inserts_functions_and_user_nodes_by_uuid() {
    let (dir, mut e) = uuid_project();
    open_rock_wet(&mut e);
    let pick = |e: &mut Editor, kind: &str| -> NodeId {
        let entries = super::palette::palette_entries(e, None);
        let entry = entries.iter().find(|x| x.id == kind).unwrap_or_else(|| panic!("no entry {kind}")).clone();
        assert!(entry.disabled.is_none(), "{kind}");
        added_by(e, (entry.pick)(bevy::math::Vec2::new(10.0, 20.0)))
    };
    let node = pick(&mut e, "user::soft_clamp");
    assert_eq!(kinds(&e, &node), (format!("user::{}", user_uuid("soft_clamp")), "user::soft_clamp".into()));
    let call = pick(&mut e, "function::common/wet_mask");
    let stored = format!("function::{}", uuid_of(AssetType::Function, "common/wet_mask"));
    assert_eq!(kinds(&e, &call), (stored, "function::common/wet_mask".into()));

    // A hand-written node without an id: disabled, picking it adds nothing.
    let file = dir.path().join("quillshade/material/.nodes/hand.ron");
    let hand = "UserNode(format_version: 2, ident: \"hand_clamp\", name: \"HandClamp\", inputs: [UserPin(name: \"x\", \
                ty: Float)], output: Float, template: \"return x;\")\n";
    std::fs::write(&file, hand).unwrap();
    e.files_changed(vec![file.clone()]);
    let entries = super::palette::palette_entries(&e, None);
    let entry = entries.iter().find(|x| x.id == "user::hand_clamp").expect("listed");
    assert_eq!(entry.disabled.as_deref(), Some(crate::graph_domain::NEEDS_UUID));
    // Assign UUID: then it is inserted by its new UUID.
    let ae004 = e.console_diagnostics().into_iter().find(|d| d.code == "AE004").expect("AE004");
    e.apply(Action::FocusDiagnostic(ae004));
    e.apply(confirm(&e));
    let id = e.asset_index.id_of(&assets::user_node_ref("hand")).expect("an id");
    let node = pick(&mut e, "user::hand_clamp");
    assert_eq!(kinds(&e, &node), (format!("user::{id}"), "user::hand_clamp".into()));
}

/// The right sidebar's pickers offer the targets' UUIDs labelled by path, show the stored UUID as its target, and
/// picking stores the UUID; picking what the node shows changes nothing (no undo step, the slot stays).
#[test]
fn pickers_store_the_target_uuid() {
    let (_dir, mut e) = uuid_project();
    let pulse = uuid_of(AssetType::Curve, "common/pulse");
    let steps = uuid_of(AssetType::Curve, "common/steps");
    let (choices, current) = crate::editors::graph::reference_choices(&e, AssetType::Curve, "common/pulse");
    assert_eq!(current, pulse);
    assert_eq!(choices, [(pulse.clone(), "common/pulse".into()), (steps.clone(), "common/steps".into())]);
    let (choices, current) = crate::editors::graph::reference_choices(&e, AssetType::Texture, DEFAULT_TEXTURE);
    assert_eq!(current, DEFAULT_TEXTURE);
    let rock = uuid_of(AssetType::Texture, "textures/rock_albedo");
    assert!(choices.contains(&(rock, "textures/rock_albedo".into())), "{choices:?}");
    let gone = project_core::AssetUuid::new_v7().to_string();
    let (choices, current) = crate::editors::graph::reference_choices(&e, AssetType::Curve, &gone);
    assert_eq!(
        (current.as_str(), choices.last().unwrap()),
        (gone.as_str(), &(gone.clone(), format!("{gone} (missing)")))
    );

    e.apply(Action::OpenAsset(graph_id("fx/curve_bands", false).to_ref()));
    let sample = node_of_kind(&e, "stdlib::SampleCurve");
    let curve = GraphProp::Param { node: sample.clone(), key: "curve".into() };
    let revision = e.doc().unwrap().revision;
    set(&mut e, curve.clone(), Value::Str(pulse.clone()));
    assert!(!e.doc().unwrap().is_dirty(), "the node shows that curve already");
    assert!(!e.doc().unwrap().can_undo() && e.doc().unwrap().revision == revision, "no edit");
    set(&mut e, curve.clone(), Value::Str(steps.clone()));
    assert_eq!(stored_param(&e, &sample, "curve"), Some(Value::Str(steps)));
    assert_eq!(shown(&e, &curve), Some(Value::Str("common/steps".into())));

    // Picking the collection a Sample Gradient shows keeps its slot.
    e.apply(Action::OpenAsset(graph_id("env/terrain_layers", false).to_ref()));
    let sample = node_of_kind(&e, "stdlib::SampleGradient");
    let slot = e.referenced_gradient_slot(&sample).expect("a slot");
    let key = GraphProp::Param { node: sample.clone(), key: GRADIENT_COLLECTION_PARAM.into() };
    set(&mut e, key, Value::Str(uuid_of(AssetType::Gradients, "common/terrain")));
    assert_eq!(e.referenced_gradient_slot(&sample), Some(slot));
    assert!(!e.doc().unwrap().is_dirty());
}

/// Saving a graph writes the D27 path comment after each UUID reference (open tab, and graphs a constant type
/// change rewrites on disk).
#[test]
fn saves_annotate_uuid_references() {
    let (dir, mut e) = uuid_project();
    e.apply(Action::OpenAsset(graph_id("fx/curve_bands", false).to_ref()));
    let sample = node_of_kind(&e, "stdlib::SampleCurve");
    let curve = GraphProp::Param { node: sample, key: "curve".into() };
    set(&mut e, curve, Value::Str(uuid_of(AssetType::Curve, "common/steps")));
    e.apply(Action::Save);
    let text = std::fs::read_to_string(dir.path().join("quillshade/material/fx/curve_bands.material.ron")).unwrap();
    let steps = uuid_of(AssetType::Curve, "common/steps");
    assert!(text.contains(&format!("\"{steps}\"), // common/steps\n")), "{text}");
    assert!(!e.is_dirty());

    // A confirmed constant type change rewrites the closed `fx/global_tint` annotated.
    e.apply(Action::OpenAsset(tuning().to_ref()));
    set(&mut e, ConstsProp::Type(GLOW_AMOUNT_ID), Value::Str("vec2".into()));
    e.apply(confirm(&e));
    let text = std::fs::read_to_string(global_tint().file(e.project.as_ref().unwrap())).unwrap();
    assert!(text.contains("), // common/tuning\n"), "{text}");
}

/// ADR 0047 §6: a new graph gets a UUIDv7 on its first save, later saves keep it, and the tab stays clean.
#[test]
fn first_save_mints_an_id_and_later_saves_keep_it() {
    let (dir, mut e) = uuid_project();
    e.apply(new_asset("material", 0));
    e.apply(Action::Submit("fx/fresh".into()));
    let id = graph_id("fx/fresh", false);
    assert_eq!(e.doc().unwrap().id, id);
    let i = e.active.unwrap();
    assert!(e.save(i));
    let minted = e.doc().unwrap().asset.id;
    assert!(minted.is_v7(), "{minted}");
    assert!(!e.doc().unwrap().is_dirty(), "the saved state has the id");
    let file = dir.path().join(id.source_rel());
    assert_eq!(assets::load(e.project.as_ref().unwrap(), &id).unwrap().id, minted);
    e.doc_mut().unwrap().edit(|a| a.meta.description = "fresh".into());
    e.apply(Action::Save);
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(text.contains(&format!("\n  id: \"{minted}\",\n")) && text.contains("fresh"), "{text}");
    assert_eq!(e.asset_index.id_of(&id.to_ref()), Some(minted), "the index knows it after the save");
}

// ---- rename / move / delete (ADR 0047 §13: no reference is rewritten) ----

/// Every source file under `quillshade/` of the project in `dir` but `skip` (paths relative to `dir`).
fn sources_but(dir: &Path, skip: &[&str]) -> std::collections::BTreeMap<String, Vec<u8>> {
    fn walk(dir: &Path, root: &Path, out: &mut std::collections::BTreeMap<String, Vec<u8>>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, root, out);
            } else {
                out.insert(
                    path.strip_prefix(root).unwrap().to_string_lossy().into_owned(),
                    std::fs::read(&path).unwrap(),
                );
            }
        }
    }
    let mut out = std::collections::BTreeMap::new();
    walk(&dir.join("quillshade"), dir, &mut out);
    out.retain(|rel, _| !skip.contains(&rel.as_str()));
    out
}

/// The title the canvas shows for `node` of the active graph.
fn title_of(e: &Editor, node: &str) -> String {
    let view = e.view_of(e.doc().unwrap());
    crate::doc::node_title(&e.domains.material().registry, &e.domains.material().functions, &view, node)
}

/// An in-editor rename of a curve and of a function changes no other file: the open referring graphs stay clean,
/// store the same UUIDs and show the new paths, the project regenerates cleanly into the new module paths.
#[test]
fn rename_keeps_referrers_byte_identical() {
    let (dir, mut e) = uuid_project();
    let p = e.project.clone().unwrap();
    e.apply(Action::OpenAsset(graph_id("env/rock_wet", false).to_ref()));
    let call = node_of_kind(&e, "function::common/wet_mask");
    e.apply(Action::OpenAsset(graph_id("fx/curve_bands", false).to_ref()));
    let sample = node_of_kind(&e, "stdlib::SampleCurve");
    let stored = stored_param(&e, &sample, "curve");
    assert!(title_of(&e, &sample).contains("pulse"), "{}", title_of(&e, &sample));

    let curve = AssetId::new(AssetType::Curve, "common/pulse");
    let before = sources_but(dir.path(), &[&curve.source_rel()]);
    e.rename(&curve.to_ref(), "common/pulse2");
    let moved = AssetId::new(AssetType::Curve, "common/pulse2");
    assert_eq!(sources_but(dir.path(), &[&moved.source_rel()]), before, "no other file changes");
    assert!(!e.doc().unwrap().is_dirty());
    assert_eq!(stored_param(&e, &sample, "curve"), stored, "the graph keeps the UUID");
    assert_eq!(param_of(&e, &sample, "curve"), Some(Value::Str("common/pulse2".into())));
    assert!(title_of(&e, &sample).contains("pulse2"), "{}", title_of(&e, &sample));
    assert!(wesl_file(&p, &moved).unwrap().exists() && !wesl_file(&p, &curve).unwrap().exists());

    let function = graph_id("common/wet_mask", true);
    let before = sources_but(dir.path(), &[&function.source_rel()]);
    e.rename(&function.to_ref(), "lib/wet_mask");
    let moved = graph_id("lib/wet_mask", true);
    assert_eq!(sources_but(dir.path(), &[&moved.source_rel()]), before, "no other file changes");
    e.apply(Action::OpenAsset(graph_id("env/rock_wet", false).to_ref()));
    assert!(!e.doc().unwrap().is_dirty());
    assert_eq!(e.view_of(e.doc().unwrap()).nodes[&call].kind, "function::lib/wet_mask");
    let wesl = std::fs::read_to_string(wesl_file(&p, &graph_id("env/rock_wet", false)).unwrap()).unwrap();
    assert!(wesl.contains("functions::dir_lib::fun_wet_mask"), "{wesl}");
    e.validate();
    let errors: Vec<_> = e.all_diagnostics().into_iter().filter(|d| d.is_error()).collect();
    assert!(errors.is_empty() && e.index_diagnostics.is_empty(), "{errors:?} {:?}", e.index_diagnostics);
}

/// Files renamed / moved outside the editor (a curve, a user node file) reach it through the watcher: the index is
/// rebuilt, nothing is reported, the open graph shows the new path, and Generate writes the new module path.
#[test]
fn external_rename_keeps_references_valid() {
    let (dir, mut e) = uuid_project();
    let p = e.project.clone().unwrap();
    e.apply(Action::OpenAsset(graph_id("fx/user_node", false).to_ref()));
    let user = node_of_kind(&e, "user::soft_clamp");
    e.apply(Action::OpenAsset(graph_id("fx/curve_bands", false).to_ref()));
    let sample = node_of_kind(&e, "stdlib::SampleCurve");
    let material = p.material_dir();
    let moves = [("common/pulse.curve.ron", "ramps/pulse.curve.ron"), (".nodes/soft_clamp.ron", ".nodes/clamp.ron")];
    let mut changed = Vec::new();
    for (from, to) in moves {
        std::fs::create_dir_all(material.join(to).parent().unwrap()).unwrap();
        std::fs::rename(material.join(from), material.join(to)).unwrap();
        changed.extend([material.join(from), material.join(to)]);
    }
    let before = sources_but(dir.path(), &[]);
    e.files_changed(changed);
    assert_eq!(sources_but(dir.path(), &[]), before, "nothing is rewritten");
    let pulse = AssetId::new(AssetType::Curve, "ramps/pulse").to_ref();
    assert_eq!(e.asset_index.id_of(&pulse).map(|u| u.to_string()), Some(uuid_of(AssetType::Curve, "common/pulse")));
    assert!(e.index_diagnostics.is_empty(), "{:?}", e.index_diagnostics);
    assert!(!e.doc().unwrap().is_dirty());
    assert_eq!(param_of(&e, &sample, "curve"), Some(Value::Str("ramps/pulse".into())));
    e.apply(Action::OpenAsset(graph_id("fx/user_node", false).to_ref()));
    assert_eq!(e.view_of(e.doc().unwrap()).nodes[&user].kind, "user::soft_clamp", "the user node still resolves");
    e.apply(Action::RegenAll);
    let errors: Vec<_> = e.all_diagnostics().into_iter().filter(|d| d.is_error()).collect();
    assert!(errors.is_empty(), "{errors:?}");
    let bands = std::fs::read_to_string(wesl_file(&p, &graph_id("fx/curve_bands", false)).unwrap()).unwrap();
    assert!(bands.contains("curves::dir_ramps::crv_pulse::"), "{bands}");
}

/// A file copied outside the editor shares its original's UUID: AE001 at the copy in the console, references keep
/// resolving to the original.
#[test]
fn copied_file_is_reported_as_duplicate() {
    let (_dir, mut e) = uuid_project();
    let material = e.project.as_ref().unwrap().material_dir();
    e.apply(Action::OpenAsset(graph_id("fx/curve_bands", false).to_ref()));
    let sample = node_of_kind(&e, "stdlib::SampleCurve");
    let copy = material.join("common/pulse_copy.curve.ron");
    std::fs::copy(material.join("common/pulse.curve.ron"), &copy).unwrap();
    e.files_changed(vec![copy]);
    let ae: Vec<_> = e.console_diagnostics().into_iter().filter(|d| d.code == "AE001").collect();
    assert_eq!(ae.len(), 1, "{ae:?}");
    assert_eq!(ae[0].asset.as_deref(), Some("quillshade/material/common/pulse_copy.curve.ron"));
    assert_eq!(param_of(&e, &sample, "curve"), Some(Value::Str("common/pulse".into())));
    assert!(e.error_count() >= 1);
}

/// Deleting a copied file (a duplicate loser, AE001) leaves the references to its original alone, on disk and in
/// open tabs (clean or dirty): they resolve to the original (ADR 0047 §8). Deleting the original still clears them.
#[test]
fn deleting_a_duplicate_copy_keeps_refs_to_the_original() {
    let (_dir, mut e) = uuid_project();
    let p = e.project.clone().unwrap();
    let kinds = [
        (AssetType::Texture, "textures/rock_albedo"),
        (AssetType::Gradients, "common/terrain"),
        (AssetType::Consts, "common/tuning"),
    ];
    let mut copies = Vec::new();
    let mut users = BTreeSet::new();
    for (ty, path) in kinds {
        let original = AssetId::new(ty, path);
        let copy = AssetId::new(ty, format!("{path}_copy"));
        std::fs::copy(original.file(&p), copy.file(&p)).unwrap();
        e.files_changed(vec![copy.file(&p)]);
        let uuid = e.asset_index.ref_id_of(&original.to_ref()).expect("the original is the winner");
        let referrers: Vec<_> = e.asset_index.refs_to(uuid).filter_map(AssetId::from_ref).collect();
        assert!(!referrers.is_empty(), "{path} is referenced");
        users.extend(referrers);
        copies.push(copy);
    }
    for id in &users {
        e.apply(Action::OpenAsset(id.to_ref()));
    }
    // One referrer is dirty: unsaved edits stay, the references too.
    let dirty = users.first().unwrap().clone();
    let i = e.doc_index(&dirty).unwrap();
    e.tabs[i].get_mut::<Doc>().unwrap().edit(|a| a.nodes.values_mut().next().unwrap().pos.0 += 10.0);
    let snapshot = |e: &Editor| -> Vec<(Vec<u8>, Asset, bool)> {
        users
            .iter()
            .map(|id| {
                let d = e.tabs[e.doc_index(id).unwrap()].get::<Doc>().unwrap();
                (std::fs::read(id.file(&p)).unwrap(), d.asset.clone(), d.is_dirty())
            })
            .collect()
    };
    let before = snapshot(&e);
    for copy in &copies {
        e.apply(Action::Delete(copy.to_ref()));
        assert!(!copy.file(&p).exists());
        assert!(snapshot(&e) == before, "deleting {} changed a referrer of its original", copy.path);
    }
    let tex = AssetId::new(AssetType::Texture, "textures/rock_albedo");
    e.apply(Action::Delete(tex.to_ref()));
    assert!(snapshot(&e) != before, "deleting the original clears its references");
}

/// ADR 0047 §8: a copied file shares its original's UUID; reference pickers offer the original only (picking the
/// copy would store the original's UUID), and a copy cannot be referenced by path either.
#[test]
fn pickers_skip_duplicate_copies() {
    let (_dir, mut e) = uuid_project();
    let p = e.project.clone().unwrap();
    for (ty, path) in [
        (AssetType::Texture, "textures/rock_albedo"),
        (AssetType::Gradients, "common/terrain"),
        (AssetType::Consts, "common/tuning"),
        (AssetType::Curve, "common/pulse"),
    ] {
        let copy = AssetId::new(ty, format!("{path}_copy"));
        std::fs::copy(AssetId::new(ty, path).file(&p), copy.file(&p)).unwrap();
        e.files_changed(vec![copy.file(&p)]);
        let (choices, _) = crate::editors::graph::reference_choices(&e, ty, "");
        assert!(choices.contains(&(uuid_of(ty, path), path.to_string())), "{choices:?}");
        assert!(choices.iter().all(|c| !c.1.contains(&copy.path)), "{choices:?}");
        assert_eq!(e.stored_ref(ty, &copy.path), None, "{}", copy.path);
    }
}

/// ADR 0047 §6: the first save of a file without an `id` mints one in place, without an undo step; undoing an edit
/// made before that save keeps it, so the next save writes the same UUID (references stored meanwhile keep
/// resolving). Every Material document kind: graph, curve, constant table, gradient collection, Texture asset.
#[test]
fn undo_after_first_save_keeps_the_minted_uuid() {
    use crate::texture::MaterialTexture;
    use texture_kit::editor::TextureDoc;
    let (_dir, mut e) = uuid_project();
    let p = e.project.clone().unwrap();
    let ids = [
        AssetId::new(AssetType::Material, "env/rock_wet"),
        AssetId::new(AssetType::Curve, "common/pulse"),
        AssetId::new(AssetType::Consts, "common/tuning"),
        AssetId::new(AssetType::Gradients, "common/terrain"),
        AssetId::new(AssetType::Texture, "textures/rock_albedo"),
    ];
    for id in ids {
        let file = id.file(&p);
        let text = std::fs::read_to_string(&file).unwrap();
        let bare: String = text.lines().filter(|l| !l.starts_with("  id: \"")).map(|l| format!("{l}\n")).collect();
        assert_ne!(bare, text, "{id}: no root id line");
        std::fs::write(&file, bare).unwrap();
        e.files_changed(vec![file.clone()]);
        assert_eq!(e.asset_index.id_of(&id.to_ref()), None, "{id}");
        e.apply(Action::OpenAsset(id.to_ref()));
        match id.ty {
            AssetType::Material => e.active_doc_mut::<Doc>().unwrap().edit(|a| {
                a.nodes.values_mut().next().unwrap().pos.0 += 10.0;
            }),
            AssetType::Curve => e.active_doc_mut::<CurveDoc>().unwrap().add_point((0.25, 0.75)),
            AssetType::Consts => {
                e.active_doc_mut::<crate::consts_doc::ConstsDoc>().unwrap().add("extra", ConstType::F32);
            }
            AssetType::Gradients => e.active_doc_mut::<GradientsDoc>().unwrap().set_name("Renamed".into(), None),
            _ => {
                let d = e.active_doc_mut::<TextureDoc<MaterialTexture>>().unwrap();
                let mut settings = d.asset.settings.clone();
                settings.is_srgb = !settings.is_srgb;
                d.set_settings(settings, None);
            }
        }
        e.apply(Action::Save);
        let minted = e.asset_index.id_of(&id.to_ref()).unwrap_or_else(|| panic!("{id}: the save mints an id"));
        e.apply(Action::Tab(TabAction::Undo));
        e.apply(Action::Save);
        assert_eq!(e.asset_index.id_of(&id.to_ref()), Some(minted), "{id}: the UUID stays");
        assert!(std::fs::read_to_string(&file).unwrap().contains(&minted.to_string()), "{id}");
        e.apply(Action::Tab(TabAction::Redo));
        e.apply(Action::Save);
        assert_eq!(e.asset_index.id_of(&id.to_ref()), Some(minted), "{id}: redo keeps it too");
    }
}
