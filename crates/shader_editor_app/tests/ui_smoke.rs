//! Canvas smoke tests without a window (spec §15 "UI"): create a node, connect, undo, save —
//! through the bevy_noodle API, as the user's gestures would.

use bevy::prelude::*;
use bevy_noodle::prelude::*;
use bevy_noodle::style::kit;
use shader_editor_app::EditorCorePlugin;
use shader_editor_app::canvas::{EditorCanvas, NodeKey, PinKey};
use shader_editor_app::state::{Action, Actions, Editor};
use shader_editor_app::ui::EditorCanvasMarker;
use shader_project::{AssetId, AssetType, Project, fixtures, ops};

fn app() -> (tempfile::TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"g\"\n").unwrap();
    std::fs::create_dir_all(dir.path().join("assets")).unwrap();
    let p = Project::open(dir.path()).unwrap();
    for (path, is_fn, a) in fixtures::all() {
        let id = AssetId::new(if is_fn { AssetType::Function } else { AssetType::Material }, path);
        ops::save(&p, &id, &a).unwrap();
    }
    let mut editor = Editor::default();
    editor.open_project(dir.path()).unwrap();

    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        AssetPlugin::default(),
        bevy::scene::ScenePlugin,
        bevy::input::InputPlugin,
        NoodleCorePlugin,
    ));
    app.insert_resource(editor);
    app.add_plugins(EditorCorePlugin);
    app.world_mut().spawn_scene(bsn! { @kit::canvas() EditorCanvasMarker }).unwrap();
    app.update();
    (dir, app)
}

fn act(app: &mut App, a: Action) {
    app.world_mut().resource_mut::<Actions>().push(a);
    app.update();
    app.update();
}

fn port(app: &mut App, node: &str, pin: &str, output: bool) -> Entity {
    let w = app.world_mut();
    let mut q = w.query::<(Entity, &PinKey)>();
    q.iter(w)
        .find(|(_, k)| k.node == node && k.pin == pin && k.output == output)
        .map(|(e, _)| e)
        .unwrap_or_else(|| panic!("no port {node}.{pin}"))
}

fn counts(app: &mut App) -> (usize, usize) {
    let w = app.world_mut();
    let nodes = w.query::<&NodeKey>().iter(w).count();
    let edges = w.query::<&Edge>().iter(w).count();
    (nodes, edges)
}

#[test]
fn canvas_mirrors_model_and_user_edits_flow_back() {
    let (dir, mut app) = app();
    assert!(app.world().contains_resource::<EditorCanvas>());
    act(&mut app, Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet")));
    let (n_model, e_model) = {
        let d = app.world().resource::<Editor>().doc().unwrap().clone();
        (d.asset.nodes.len(), d.asset.edges.len())
    };
    assert_eq!(counts(&mut app), (n_model, e_model));

    // Create a node (palette pick) and connect it by "dragging" a wire.
    act(&mut app, Action::AddNode { kind: "stdlib::Constant".into(), pos: (-400.0, 0.0), link: None });
    let editor = app.world().resource::<Editor>();
    let d = editor.doc().unwrap();
    let c =
        d.asset.nodes.iter().find(|(_, n)| n.kind == "stdlib::Constant" && n.pos == (-400.0, 0.0)).unwrap().0.clone();
    let out = d.asset.nodes.iter().find(|(_, n)| n.kind == "output::Pbr3d").unwrap().0.clone();
    assert_eq!(counts(&mut app), (n_model + 1, e_model));

    let canvas = app.world().resource::<EditorCanvas>().canvas;
    let from = port(&mut app, &c, "out", true);
    let to = port(&mut app, &out, "metallic", false);
    app.world_mut().graph_edit_with_origin(canvas, GraphEdit::Connect { from, to }, EditOrigin::Interaction).unwrap();
    app.update();
    app.update();
    let d = app.world().resource::<Editor>().doc().unwrap();
    assert_eq!(d.asset.edge_into(&out, "metallic").map(|e| e.from.0.clone()), Some(c.clone()));

    // Type check (spec §7.2): a Float2 into Base Color (Float3) is refused by `ConnectionCheck`.
    act(&mut app, Action::AddNode { kind: "stdlib::Constant2Vector".into(), pos: (-400.0, 200.0), link: None });
    let d = app.world().resource::<Editor>().doc().unwrap();
    let v2 = d.asset.nodes.iter().find(|(_, n)| n.kind == "stdlib::Constant2Vector").unwrap().0.clone();
    let from = port(&mut app, &v2, "out", true);
    let to = port(&mut app, &out, "base_color", false);
    let r = app.world_mut().graph_edit_with_origin(canvas, GraphEdit::Connect { from, to }, EditOrigin::Interaction);
    assert!(r.is_err(), "incompatible wire must be rejected");

    // Undo removes the Constant2Vector node and the canvas follows.
    act(&mut app, Action::Undo);
    assert_eq!(counts(&mut app), (n_model + 1, e_model + 1));
    act(&mut app, Action::Undo);
    let d = app.world().resource::<Editor>().doc().unwrap();
    assert!(d.asset.edge_into(&out, "metallic").is_none_or(|e| e.from.0 != c));

    // Save writes the source and regenerates.
    act(&mut app, Action::Redo);
    act(&mut app, Action::Save);
    let saved = std::fs::read_to_string(dir.path().join("shaders/env/rock_wet.material.ron")).unwrap();
    assert!(saved.contains("stdlib::Constant\""));
    assert!(!app.world().resource::<Editor>().is_dirty());
    assert!(dir.path().join("assets/shaders/autogen/env/rock_wet.wesl").is_file());
}

#[test]
fn moving_nodes_updates_model_without_rebuild() {
    let (_dir, mut app) = app();
    act(&mut app, Action::OpenAsset(AssetId::new(AssetType::Material, "fx/hologram")));
    let canvas = app.world().resource::<EditorCanvas>().canvas;
    let (entity, id, before) = {
        let w = app.world_mut();
        let mut q = w.query::<(Entity, &NodeKey, &NodePosition)>();
        let (e, k, p) = q.iter(w).next().unwrap();
        (e, k.0.clone(), p.0)
    };
    let edit = GraphEdit::MoveNodes { nodes: vec![entity], delta: Vec2::new(50.0, 20.0), drag: None };
    app.world_mut().graph_edit_with_origin(canvas, edit, EditOrigin::Interaction).unwrap();
    for _ in 0..4 {
        app.update();
    }
    let d = app.world().resource::<Editor>().doc().unwrap();
    assert_eq!(d.asset.nodes[&id].pos, (before.x + 50.0, before.y + 20.0));
    assert!(d.is_dirty());
    // Same entity: the canvas was not rebuilt for a move.
    assert!(app.world().get::<NodeKey>(entity).is_some());
}
