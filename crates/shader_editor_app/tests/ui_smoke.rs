//! Canvas smoke tests without a window (spec §15 "UI"): create a node, connect, undo, save —
//! through the bevy_noodle API, as the user's gestures would.

use bevy::prelude::*;
use bevy_noodle::prelude::*;
use bevy_noodle::style::kit;
use shader_editor_app::EditorCorePlugin;
use shader_editor_app::canvas::{EditorCanvas, NodeKey, PinKey};
use shader_editor_app::state::{Action, Actions, Editor, PropTarget};
use shader_editor_app::tab::Lang;
use shader_editor_app::ui::EditorCanvasMarker;
use shader_editor_app::ui::props::PropField;
use shader_project::{AssetId, AssetType, Project, fixtures};

fn app() -> (tempfile::TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"g\"\n").unwrap();
    std::fs::create_dir_all(dir.path().join("assets")).unwrap();
    let p = Project::open(dir.path()).unwrap();
    fixtures::save_all(&p).unwrap();
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
    // Inline editors on nodes are Feathers widgets; their templates load fonts.
    app.init_asset::<Font>().init_asset::<Image>();
    app.init_resource::<bevy::input_focus::InputFocus>().init_resource::<bevy::input_focus::InputFocusVisible>();
    app.init_resource::<bevy::feathers::controls::UnitsRegistry>();
    app.insert_resource(bevy::feathers::theme::UiTheme(bevy::feathers::dark_theme::create_dark_theme()));
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
    act(
        &mut app,
        Action::AddNode { kind: "stdlib::Constant".into(), pos: (-400.0, 0.0), link: None, params: Vec::new() },
    );
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
    act(
        &mut app,
        Action::AddNode {
            kind: "stdlib::Constant2Vector".into(),
            pos: (-400.0, 200.0),
            link: None,
            params: Vec::new(),
        },
    );
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

#[test]
fn text_tab_empties_the_canvas_and_graph_tab_restores_it() {
    let (_dir, mut app) = app();
    let id = AssetId::new(AssetType::Material, "fx/hologram");
    act(&mut app, Action::OpenAsset(id.clone()));
    let shown = counts(&mut app);
    assert!(shown.0 > 0);
    let view = |app: &App| {
        let canvas = app.world().resource::<EditorCanvas>().canvas;
        app.world().get::<CanvasView>(canvas).map(|v| (v.pan, v.zoom)).unwrap()
    };
    act(&mut app, Action::View(shader_editor_app::state::ViewRequest::FitAll));
    let fitted = view(&app);
    app.world_mut().resource_mut::<Editor>().regenerate();
    act(&mut app, Action::ShowGenerated(id.clone(), Lang::Wesl));
    assert!(app.world().resource::<Editor>().doc().is_none());
    assert_eq!(counts(&mut app), (0, 0), "a text tab shows no graph");
    // Graph commands are no-ops on a text tab.
    act(&mut app, Action::AddNode { kind: "stdlib::Time".into(), pos: (0.0, 0.0), link: None, params: Vec::new() });
    act(&mut app, Action::ActivateTab(0));
    assert_eq!(counts(&mut app), shown);
    assert_eq!(view(&app), fitted, "the graph keeps its camera");
    assert!(!app.world().resource::<Editor>().is_dirty());
}

fn added(app: &App, kind: &str, pos: (f32, f32)) -> String {
    let d = app.world().resource::<Editor>().doc().unwrap();
    d.asset.nodes.iter().find(|(_, n)| n.kind == kind && n.pos == pos).unwrap().0.clone()
}

fn has_inline(app: &mut App, node: &str, pin: &str) -> bool {
    let want = PropTarget::Input { node: node.into(), pin: pin.into() };
    let w = app.world_mut();
    w.query::<&PropField>().iter(w).any(|f| f.target == want)
}

#[test]
fn inline_editor_follows_connection() {
    let (_dir, mut app) = app();
    act(&mut app, Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet")));
    let new = |kind: &str, pos| Action::AddNode { kind: kind.into(), pos, link: None, params: Vec::new() };
    act(&mut app, new("stdlib::Lerp", (-600.0, 0.0)));
    act(&mut app, new("stdlib::Time", (-900.0, 0.0)));
    let (lerp, time) = (added(&app, "stdlib::Lerp", (-600.0, 0.0)), added(&app, "stdlib::Time", (-900.0, 0.0)));
    assert!(has_inline(&mut app, &lerp, "alpha"), "unconnected alpha shows its editor");

    act(&mut app, Action::Connect { from: (time.clone(), "out".into()), to: (lerp.clone(), "alpha".into()) });
    assert!(!has_inline(&mut app, &lerp, "alpha"), "a wire hides the editor");
    assert!(has_inline(&mut app, &lerp, "a"));

    act(&mut app, Action::Undo);
    assert!(has_inline(&mut app, &lerp, "alpha"), "disconnecting brings it back");
}

fn inline_entity(app: &mut App, node: &str, pin: &str) -> Option<Entity> {
    let want = PropTarget::Input { node: node.into(), pin: pin.into() };
    let w = app.world_mut();
    w.query::<(Entity, &PropField)>().iter(w).find(|(_, f)| f.target == want).map(|(e, _)| e)
}

/// The editable text inside widget `root`.
fn text_in(app: &mut App, root: Entity) -> Entity {
    let w = app.world_mut();
    let texts: Vec<Entity> = w.query_filtered::<Entity, With<bevy::text::EditableText>>().iter(w).collect();
    let inside = |mut e: Entity| loop {
        if e == root {
            return true;
        }
        match w.get::<ChildOf>(e) {
            Some(p) => e = p.parent(),
            None => return false,
        }
    };
    texts.into_iter().find(|&t| inside(t)).unwrap()
}

fn focus(app: &mut App, e: Entity) {
    app.world_mut().resource_mut::<bevy::input_focus::InputFocus>().set(e, bevy::input_focus::FocusCause::Pressed);
}

#[test]
fn focused_inline_text_field_defers_rebuild() {
    let (_dir, mut app) = app();
    act(&mut app, Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet")));
    act(&mut app, Action::AddNode { kind: "stdlib::Lerp".into(), pos: (-600.0, 0.0), link: None, params: Vec::new() });
    let lerp = added(&app, "stdlib::Lerp", (-600.0, 0.0));

    // A slider keeps focus after a click; that alone must not freeze the canvas.
    let slider = inline_entity(&mut app, &lerp, "alpha").unwrap();
    focus(&mut app, slider);
    let alpha = PropTarget::Input { node: lerp.clone(), pin: "alpha".into() };
    act(&mut app, Action::SetComponent { target: alpha, comp: 0, value: 0.7, is_final: true });
    assert!(app.world().get_entity(slider).is_err(), "rebuilt despite the focused slider");

    // Text being typed into a number field survives the rebuild that its own edit would cause.
    let field = inline_entity(&mut app, &lerp, "a").unwrap();
    let text = text_in(&mut app, field);
    focus(&mut app, text);
    let target = PropTarget::Input { node: lerp.clone(), pin: "a".into() };
    act(&mut app, Action::SetComponent { target, comp: 0, value: 0.3, is_final: true });
    assert!(app.world().get_entity(field).is_ok(), "the focused field survives the edit");

    app.world_mut().resource_mut::<bevy::input_focus::InputFocus>().clear();
    app.update();
    app.update();
    assert!(app.world().get_entity(field).is_err(), "the canvas catches up once focus leaves");
    assert!(inline_entity(&mut app, &lerp, "a").is_some());
}

#[test]
fn inline_fields_follow_the_model_while_rebuild_waits() {
    let (_dir, mut app) = app();
    act(&mut app, Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet")));
    act(&mut app, Action::AddNode { kind: "stdlib::Lerp".into(), pos: (-600.0, 0.0), link: None, params: Vec::new() });
    let lerp = added(&app, "stdlib::Lerp", (-600.0, 0.0));
    act(&mut app, Action::SetPinType { node: lerp.clone(), ty: shader_graph_core::PinType::Float3 });
    let a = PropTarget::Input { node: lerp.clone(), pin: "a".into() };
    let field = |app: &mut App, comp: usize| {
        let w = app.world_mut();
        let mut q = w.query_filtered::<(Entity, &PropField), With<bevy::feathers::controls::FeathersNumberInput>>();
        q.iter(w).find(|(_, f)| f.target == a && f.comp == comp).map(|(e, _)| e).unwrap()
    };
    // Typing into R defers the rebuild; G still shows what the model has.
    let r = field(&mut app, 0);
    let text = text_in(&mut app, r);
    focus(&mut app, text);
    let g = field(&mut app, 1);
    act(&mut app, Action::SetComponent { target: a.clone(), comp: 1, value: 0.42, is_final: false });
    assert!(app.world().get_entity(g).is_ok(), "not rebuilt");
    let shown = app.world().get::<bevy::feathers::controls::NumberInputValue>(g).cloned();
    assert!(matches!(shown, Some(bevy::feathers::controls::NumberInputValue::F32(v)) if v == 0.42), "{shown:?}");
}

#[test]
fn rows_without_editor_keep_their_alignment() {
    let (_dir, mut app) = app();
    act(&mut app, Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet")));
    act(&mut app, Action::AddNode { kind: "stdlib::Lerp".into(), pos: (-600.0, 0.0), link: None, params: Vec::new() });
    let lerp = added(&app, "stdlib::Lerp", (-600.0, 0.0));
    // An output row is just label + port: an empty growing editor slot would push the label off the right edge.
    let out = port(&mut app, &lerp, "out", true);
    let row = app.world().get::<ChildOf>(out).unwrap().parent();
    assert_eq!(app.world().get::<Children>(row).unwrap().len(), 2);
}
