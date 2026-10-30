//! Behaviour shared by the tab kinds: dispatch, editor statusbar, watermark, keep mine, tree state.

use super::*;

#[test]
fn tab_doc_dispatch_matches_kinds() {
    use crate::tab::MaterialKey;
    let (_dir, mut e) = project();
    e.regenerate();
    let rock = AssetId::new(AssetType::Material, "env/rock_wet");
    let pulse = AssetId::new(AssetType::Curve, "common/pulse");
    let tuning = AssetId::new(AssetType::Consts, "common/tuning");
    e.apply(Action::OpenAsset(rock.to_ref()));
    e.apply(MaterialAction::ShowGenerated(rock.clone(), WESL));
    e.apply(Action::OpenAsset(pulse.to_ref()));
    e.apply(Action::OpenAsset(tuning.to_ref()));
    let expected = [
        (
            crate::editors::graph::GRAPH,
            "quillshade/material/env/rock_wet.material",
            false,
            Some(&rock),
            MaterialKey::Asset(rock.clone()),
        ),
        (
            crate::editors::text::TEXT,
            "assets/quillshade/material/autogen/dir_env/mat_rock_wet.wesl",
            true,
            None,
            MaterialKey::Generated(rock.clone(), WESL),
        ),
        (
            crate::curve::CURVE,
            "quillshade/material/common/pulse.curve",
            false,
            Some(&pulse),
            MaterialKey::Asset(pulse.clone()),
        ),
        (
            crate::editors::consts::CONSTS,
            "quillshade/material/common/tuning.consts",
            false,
            Some(&tuning),
            MaterialKey::Asset(tuning.clone()),
        ),
    ];
    assert_eq!(e.tabs.len(), expected.len());
    for (tab, (view, title, read_only, asset, key)) in e.tabs.iter().zip(expected) {
        let doc = tab.doc();
        assert_eq!(doc.view(), view, "{title}");
        assert_eq!(doc.base_title(), title);
        assert_eq!(doc.is_read_only(), read_only, "{title}");
        assert_eq!(doc.asset().and_then(AssetId::from_ref).as_ref(), asset, "{title}");
        assert!(!doc.is_dirty(), "{title}");
        let owner = match &key {
            MaterialKey::Asset(id) | MaterialKey::Generated(id, _) => Some(id),
            MaterialKey::GameImage(_) | MaterialKey::Internal(_) => None,
        };
        assert_eq!(tab.owner().and_then(AssetId::from_ref).as_ref(), owner, "{title}");
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
    e.apply(Action::OpenAsset(rock.to_ref()));
    let status = e.status_info().expect("a graph tab has a statusbar");
    assert!(!status.dirty);
    let wesl_lines = |n| crate::util::text::plural(n, "WESL line", "WESL lines");
    let any_wesl = |c: &String| c.ends_with(" WESL line") || c.ends_with(" WESL lines");
    // Opening the project generated every domain without its module (design §6.5): the count is the file's.
    let generated = std::fs::read_to_string(super::graph::rock_wet_wesl(&e)).unwrap();
    assert_eq!(status.counts.iter().filter(|c| any_wesl(c)).count(), 1, "{:?}", status.counts);
    assert!(status.counts.contains(&wesl_lines(generated.lines().count())), "{:?}", status.counts);
    // An unconnected node: a warning of this asset.
    e.apply(MaterialAction::AddNode { kind: "stdlib::Time".into(), pos: (0.0, 0.0), link: None, params: Vec::new() });
    e.validate();
    let status = e.status_info().unwrap();
    assert!(status.dirty);
    assert_eq!(status.counts[0], crate::util::text::plural(e.doc().unwrap().asset.nodes.len(), "node", "nodes"));
    let own: Vec<&Diagnostic> = e.domains.material().diags.get(&rock).into_iter().flatten().collect();
    assert_eq!(status.errors, own.iter().filter(|d| d.is_error()).count());
    assert_eq!(status.warnings, own.iter().filter(|d| !d.is_error()).count());
    assert!(status.warnings > 0, "{own:?}");

    e.apply(Action::RegenAll);
    let wesl = std::fs::read_to_string(dir.path().join("assets/quillshade/material/autogen/dir_env/mat_rock_wet.wesl"))
        .unwrap();
    let status = e.status_info().unwrap();
    assert!(status.counts.contains(&wesl_lines(wesl.lines().count())), "{:?}", status.counts);

    // A preview writes the asset's WESL from memory (a scoped regeneration): the count follows the file.
    let lerp = add_lerp(&mut e);
    e.apply(GraphEdit::Select([lerp].into()));
    press_v(&mut e);
    let previewed = std::fs::read_to_string(rock_wet_wesl(&e)).unwrap();
    assert_ne!(previewed.lines().count(), wesl.lines().count());
    let lines = wesl_lines(previewed.lines().count());
    assert!(e.status_info().unwrap().counts.contains(&lines), "{:?}", e.status_info().unwrap().counts);
    e.apply(Action::DiscardTab(e.active.unwrap()));

    // Known for a tab opened later (the file is read once) and refreshed with a generated text tab.
    let mut next = crate::test_editor();
    next.open_project(dir.path()).unwrap();
    next.apply(Action::OpenAsset(rock.to_ref()));
    assert!(next.status_info().unwrap().counts.contains(&wesl_lines(wesl.lines().count())));
    let path = dir.path().join("assets/quillshade/material/autogen/dir_env/mat_rock_wet.wesl");
    std::fs::write(&path, format!("{wesl}// one more\n")).unwrap();
    next.apply(MaterialAction::ShowGenerated(rock.clone(), WESL));
    next.apply(Action::ActivateTab(0));
    let lines = wesl_lines(wesl.lines().count() + 1);
    assert!(next.status_info().unwrap().counts.contains(&lines), "{:?}", next.status_info().unwrap().counts);
}

#[test]
fn status_of_curve_and_consts() {
    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(pulse().to_ref()));
    let points = e.curve_doc().unwrap().curve.points.len();
    let status = e.status_info().expect("a curve tab has a statusbar");
    assert_eq!(status.counts, vec![crate::util::text::plural(points, "point", "points")]);
    assert!(!status.dirty);
    e.apply(Action::OpenAsset(tuning().to_ref()));
    let rows = e.consts_doc().unwrap().table.consts.len();
    let status = e.status_info().expect("a consts tab has a statusbar");
    assert_eq!(status.counts, vec![crate::util::text::plural(rows, "constant", "constants")]);
}

#[test]
fn no_status_for_read_only_kinds() {
    let (dir, mut e) = project();
    e.regenerate();
    let rock = AssetId::new(AssetType::Material, "env/rock_wet");
    e.apply(MaterialAction::ShowGenerated(rock.clone(), WESL));
    assert!(e.tab().and_then(Tab::get::<TextDoc>).is_some());
    assert_eq!(e.status_info(), None, "generated WESL");
    let path = "textures/env/rock.png";
    let file = dir.path().join("assets").join(path);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, b"not decoded by the editor state").unwrap();
    e.refresh_game_files();
    e.apply(Action::OpenGameImage(path.into()));
    assert!(e.tab().and_then(Tab::get::<ImageDoc>).is_some());
    assert_eq!(e.status_info(), None, "game image");
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Texture, "textures/rock_albedo").to_ref()));
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
    e.apply(Action::OpenAsset(rock.to_ref()));
    e.apply(Action::OpenAsset(mask.to_ref()));
    e.apply(MaterialAction::ShowGenerated(rock.clone(), WESL));
    e.apply(MaterialAction::ShowGenerated(rock.clone(), RUST));
    e.apply(Action::OpenAsset(pulse().to_ref()));
    e.apply(Action::OpenAsset(tuning().to_ref()));
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Texture, "textures/rock_albedo").to_ref()));
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
    e.apply(Action::OpenAsset(rock.to_ref()));
    e.apply(MaterialAction::AddNode { kind: "stdlib::Time".into(), pos: (0.0, 0.0), link: None, params: Vec::new() });
    e.apply(Action::OpenAsset(pulse.to_ref()));
    e.curve_doc_mut().unwrap().add_point((0.5, 0.5));
    // Both files change outside the editor.
    let mut asset = assets::load(&project, &rock).unwrap();
    asset.add_node(shader_graph_core::model::Node::new("stdlib::Sine", (0.0, 0.0)));
    ops::save(&project, &rock, &asset).unwrap();
    let mut curve = assets::load_curve(&project, &pulse).unwrap();
    curve.points.push((0.25, 0.75));
    curve.normalize();
    ops::save_curve(&project, &pulse, &curve).unwrap();
    let (rock_edit, pulse_edit) = (e.tabs[0].get::<Doc>().unwrap().asset.clone(), e.curve_doc().unwrap().curve.clone());
    e.apply(Action::KeepMine(asset_key(&rock)));
    e.apply(Action::KeepMine(asset_key(&pulse)));
    let (doc, curve_doc) = (e.tabs[0].get::<Doc>().unwrap(), e.curve_doc().unwrap());
    assert_eq!(doc.saved, asset.to_ron());
    assert_eq!(curve_doc.saved, curve);
    assert_eq!((&doc.asset, &curve_doc.curve), (&rock_edit, &pulse_edit), "the edits stay");
    assert!(e.tabs.iter().all(|t| t.doc().is_dirty()));
}

#[test]
fn collapse_state_is_per_tree() {
    use crate::ui::tree::{ProjectTree, TreeItem, TreeSource};
    let (_dir, mut e) = project();
    let rock = TreeItem::Asset(AssetId::new(AssetType::Material, "env/rock_wet").to_ref());
    let rows = |e: &Editor| ProjectTree(MATERIAL).rows(e).into_iter().map(|(_, item)| item).collect::<Vec<_>>();
    assert!(rows(&e).contains(&rock));

    e.apply(Action::ToggleFolder(TreeId::Project(MATERIAL), "env".into()));
    assert!(e.is_collapsed(TreeId::Project(MATERIAL), "env"));
    assert!(!e.is_collapsed(TreeId::GameAssets, "env"));
    assert!(!e.hidden_in(TreeId::GameAssets, "env/rock.png"));
    let collapsed = rows(&e);
    assert!(collapsed.contains(&TreeItem::Folder("env".into())));
    assert!(!collapsed.contains(&rock));

    // Collapsing the same path in the other tree leaves the Project tree alone.
    e.apply(Action::ToggleFolder(TreeId::GameAssets, "env".into()));
    e.apply(Action::ToggleFolder(TreeId::Project(MATERIAL), "env".into()));
    assert!(e.is_collapsed(TreeId::GameAssets, "env"));
    assert!(!e.is_collapsed(TreeId::Project(MATERIAL), "env"));
    assert!(rows(&e).contains(&rock));
}

#[test]
fn conversion_copies_params_and_generates_unique_name() {
    let (_dir, mut e) = project();
    let lerp = add_lerp(&mut e);
    let add = |e: &mut Editor, kind: &str, params: Vec<(&str, Value)>| {
        let before: Vec<_> = e.doc().unwrap().asset.nodes.keys().cloned().collect();
        let params = params.into_iter().map(|(k, v)| (k.to_string(), v)).collect();
        e.apply(MaterialAction::AddNode { kind: kind.into(), pos: (0.0, 0.0), link: None, params });
        e.doc().unwrap().asset.nodes.keys().find(|k| !before.contains(k)).unwrap().clone()
    };
    for n in ["x", "x_1"] {
        add(&mut e, "param::Scalar", vec![("name", Value::Str(n.into()))]);
    }
    let src = add(&mut e, "param::Scalar", vec![("name", Value::Str("old".into())), ("default", Value::Float(0.7))]);
    e.apply(GraphEdit::Connect { from: (src.clone(), "out".into()), to: (lerp.clone(), "a".into()) });
    let original = e.doc().unwrap().asset.nodes[&src].clone();
    let conv = Conversion {
        kind: "param::Color".into(),
        params: vec![("group".into(), "group".into())],
        unique_name: Some(("name".into(), "x".into())),
        pins: vec![("out".into(), "r".into())],
    };
    e.apply(set_value(GraphProp::Param { node: src.clone(), key: "group".into() }, Value::Str("g".into()), false));
    let original =
        shader_graph_core::model::Node { params: e.doc().unwrap().asset.nodes[&src].params.clone(), ..original };
    e.apply(MaterialAction::ConvertNodes { nodes: vec![src.clone()], conv });
    let d = e.doc().unwrap();
    let n = &d.asset.nodes[&src];
    assert_eq!(n.kind, "param::Color");
    assert_eq!(n.params["name"], Value::Str("x_2".into()));
    assert_eq!(n.params["group"], Value::Str("g".into()), "a mapped param is carried");
    assert!(!n.params.contains_key("default"), "unlisted params are dropped");
    let edge = d.asset.edge_into(&lerp, "a").expect("the edge on the renamed pin survives");
    assert_eq!(edge.from, (src.clone(), "r".to_string()));
    e.apply(Action::Tab(TabAction::Undo));
    assert_eq!(e.doc().unwrap().asset.nodes[&src], original);
    assert_eq!(e.doc().unwrap().asset.edge_into(&lerp, "a").unwrap().from.1, "out");
}

/// A generated file's tab follows its renamed source: same language, the new asset's file (decision 0014; core's
/// `TextDoc::rename` leaves the retargeting to the domain).
#[test]
fn generated_file_tab_follows_a_rename() {
    let (_dir, mut e) = project();
    e.regenerate();
    let rock = AssetId::new(AssetType::Material, "env/rock_wet");
    e.apply(MaterialAction::ShowGenerated(rock.clone(), WESL));
    e.apply(MaterialAction::ShowGenerated(rock.clone(), RUST));
    e.rename(&rock.to_ref(), "env/rock_dry");
    let moved = AssetId::new(AssetType::Material, "env/rock_dry");
    let texts: Vec<(project_core::AssetRef, &str, String)> =
        e.tabs.iter().filter_map(Tab::get::<TextDoc>).map(|t| (t.source.clone(), t.lang, t.rel.clone())).collect();
    assert_eq!(texts.len(), 2);
    for (source, lang, rel) in texts {
        assert_eq!(source, moved.to_ref());
        assert!(rel.contains("rock_dry") && !rel.contains("rock_wet"), "{lang}: {rel}");
        assert!(rel.ends_with(if lang == WESL { ".wesl" } else { ".rs" }), "{lang}: {rel}");
    }
    let keys: Vec<String> = e.tabs.iter().filter_map(|t| t.doc().config_key()).collect();
    assert!(keys.contains(&"quillshade/material/env/rock_dry.material.ron#wesl".to_string()), "{keys:?}");
}
