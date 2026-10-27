//! Canvas smoke tests without a window (spec §15 "UI"): create a node, connect, undo, save —
//! through the bevy_noodle API, as the user's gestures would.

use bevy::prelude::*;
use bevy_noodle::prelude::*;
use bevy_noodle::style::kit;
use shader_editor_app::EditorCorePlugin;
use shader_editor_app::canvas::{EditorCanvas, NodeKey, PinHalo, PinHovered, PinKey};
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
    assert!(dir.path().join("assets/shaders/autogen/dir_env/mat_rock_wet.wesl").is_file());
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

/// Design §4.1: a pointer drag snaps the lead node to the 5-unit grid at every step (visibly, not on release);
/// the other moved nodes follow by the same delta.
#[test]
fn dragging_snaps_lead_node_to_grid() {
    let (_dir, mut app) = app();
    act(&mut app, Action::OpenAsset(AssetId::new(AssetType::Material, "fx/hologram")));
    let canvas = app.world().resource::<EditorCanvas>().canvas;
    let mut ents: Vec<(Entity, String)> = {
        let w = app.world_mut();
        w.query::<(Entity, &NodeKey)>().iter(w).map(|(e, k)| (e, k.0.clone())).collect()
    };
    ents.sort_by(|a, b| a.1.cmp(&b.1));
    let (lead, other) = (ents[0].0, ents[1].0);
    // Off-grid start, like a node of an older file.
    app.world_mut().get_mut::<NodePosition>(lead).unwrap().0 = Vec2::new(12.3, 7.6);
    let pos = |app: &App, e: Entity| app.world().get::<NodePosition>(e).unwrap().0;
    let rel = pos(&app, other) - pos(&app, lead);
    let step = |app: &mut App, delta: Vec2, total: Vec2, is_final: bool| {
        let drag = Some(DragProgress { total, is_final });
        let edit = GraphEdit::MoveNodes { nodes: vec![lead, other], delta, drag };
        app.world_mut().graph_edit_with_origin(canvas, edit, EditOrigin::Interaction).unwrap();
    };
    step(&mut app, Vec2::new(1.0, 0.5), Vec2::new(1.0, 0.5), false);
    assert_eq!(pos(&app, lead), Vec2::new(15.0, 10.0), "snaps on the first step");
    step(&mut app, Vec2::new(1.0, 0.5), Vec2::new(2.0, 1.0), false);
    assert_eq!(pos(&app, lead), Vec2::new(15.0, 10.0), "stays until the next grid point is nearer");
    step(&mut app, Vec2::new(20.0, -30.0), Vec2::new(22.0, -29.0), false);
    assert_eq!(pos(&app, lead), Vec2::new(35.0, -20.0));
    step(&mut app, Vec2::ZERO, Vec2::new(22.0, -29.0), true);
    assert!((pos(&app, other) - pos(&app, lead) - rel).length() < 1e-3, "relative layout kept");
    for _ in 0..4 {
        app.update();
    }
    let d = app.world().resource::<Editor>().doc().unwrap();
    assert_eq!(d.asset.nodes[&ents[0].1].pos, (35.0, -20.0));
}

/// A pointer drag start, as bevy_picking sends it before the first drag step of a gesture.
fn drag_start(app: &mut App, entity: Entity) {
    use bevy::camera::NormalizedRenderTarget;
    use bevy::picking::backend::HitData;
    use bevy::picking::events::{Pointer, PointerDragStart};
    use bevy::picking::pointer::{Location, PointerButton, PointerId};
    let target = NormalizedRenderTarget::None { width: 800, height: 600 };
    let pointer = Pointer::new(PointerId::Mouse, Location { target, position: Vec2::ZERO });
    let hit = HitData::new(Entity::PLACEHOLDER, 0.0, None, None);
    app.world_mut().trigger(PointerDragStart { entity, pointer, button: PointerButton::Primary, hit });
}

/// The pointer entering (`over`) or leaving a port, as bevy_picking sends it (also while a wire is dragged).
fn hover(app: &mut App, entity: Entity, over: bool) {
    use bevy::camera::NormalizedRenderTarget;
    use bevy::picking::backend::HitData;
    use bevy::picking::events::{Pointer, PointerOut, PointerOver};
    use bevy::picking::pointer::{Location, PointerId};
    let target = NormalizedRenderTarget::None { width: 800, height: 600 };
    let pointer = Pointer::new(PointerId::Mouse, Location { target, position: Vec2::ZERO });
    let hit = HitData::new(Entity::PLACEHOLDER, 0.0, None, None);
    if over {
        app.world_mut().trigger(PointerOver { entity, pointer, hit });
    } else {
        app.world_mut().trigger(PointerOut { entity, pointer, hit });
    }
    app.update();
}

/// Design §4.8: a hovered pin shows its halo over the dot (1.3x, lighter); bevy_noodle's own port look is untouched.
#[test]
fn pin_hover_shows_the_halo() {
    let (_dir, mut app) = app();
    act(&mut app, Action::OpenAsset(AssetId::new(AssetType::Material, "fx/hologram")));
    let pin = {
        let w = app.world_mut();
        w.query_filtered::<Entity, With<PinKey>>().iter(w).next().unwrap()
    };
    let halo =
        app.world().get::<Children>(pin).and_then(|c| c.iter().find(|e| app.world().get::<PinHalo>(*e).is_some()));
    let halo = halo.expect("every pin has a halo child");
    let shown = |app: &App| *app.world().get::<Visibility>(halo).unwrap() != Visibility::Hidden;
    assert!(!shown(&app));
    let port_fill = app.world().get::<BackgroundColor>(pin).unwrap().0;
    hover(&mut app, pin, true);
    assert!(app.world().get::<PinHovered>(pin).is_some());
    assert!(shown(&app));
    assert!((app.world().get::<UiTransform>(halo).unwrap().scale.x - 1.3).abs() < 1e-6);
    let halo_fill = app.world().get::<BackgroundColor>(halo).unwrap().0;
    assert!(halo_fill.luminance() > port_fill.luminance(), "lighter than the dot");
    assert_eq!(app.world().get::<BackgroundColor>(pin).unwrap().0, port_fill, "the port itself is not restyled");
    hover(&mut app, pin, false);
    assert!(app.world().get::<PinHovered>(pin).is_none());
    assert!(!shown(&app));
}

/// A drag that never ends (a cancelled touch sends no final step) must not leak its start into the next gesture of
/// the same node: a new pointer drag start, or any other edit in between, begins a fresh snap drag.
#[test]
fn snap_drag_restarts_per_gesture() {
    let (_dir, mut app) = app();
    act(&mut app, Action::OpenAsset(AssetId::new(AssetType::Material, "fx/hologram")));
    let canvas = app.world().resource::<EditorCanvas>().canvas;
    let lead = {
        let w = app.world_mut();
        w.query_filtered::<Entity, With<NodeKey>>().iter(w).next().unwrap()
    };
    app.world_mut().get_mut::<NodePosition>(lead).unwrap().0 = Vec2::new(12.3, 7.6);
    let pos = |app: &App| app.world().get::<NodePosition>(lead).unwrap().0;
    let step = |app: &mut App, total: Vec2, drag: bool| {
        let drag = drag.then_some(DragProgress { total, is_final: false });
        let edit = GraphEdit::MoveNodes { nodes: vec![lead], delta: total, drag };
        app.world_mut().graph_edit_with_origin(canvas, edit, EditOrigin::Interaction).unwrap();
    };
    // An abandoned drag: the lead sits at (15, 10), no final step.
    step(&mut app, Vec2::new(1.0, 0.5), true);
    assert_eq!(pos(&app), Vec2::new(15.0, 10.0));
    // The next gesture starts from where the node is now, not from the old start (12.3, 7.6) → (20, 10).
    drag_start(&mut app, lead);
    step(&mut app, Vec2::new(10.0, 0.0), true);
    assert_eq!(pos(&app), Vec2::new(25.0, 10.0), "a new gesture snaps from the current position");
    // Abandoned again; a move that is not a drag step in between also ends it.
    step(&mut app, Vec2::new(0.0, 5.0), false);
    assert_eq!(pos(&app), Vec2::new(25.0, 15.0));
    step(&mut app, Vec2::new(10.0, 0.0), true);
    assert_eq!(pos(&app), Vec2::new(35.0, 15.0));
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
    act(&mut app, Action::SetPinType { node: lerp.clone(), ty: shader_graph_core::PinType::Float2 });
    let a = PropTarget::Input { node: lerp.clone(), pin: "a".into() };
    let field = |app: &mut App, comp: usize| {
        let w = app.world_mut();
        let mut q = w.query_filtered::<(Entity, &PropField), With<bevy::feathers::controls::FeathersNumberInput>>();
        q.iter(w).find(|(_, f)| f.target == a && f.comp == comp).map(|(e, _)| e).unwrap()
    };
    // Typing into X defers the rebuild; Y still shows what the model has.
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

#[test]
fn node_previews_follow_the_model_without_rebuild() {
    use shader_editor_app::ui::const_preview::{ConstPart, ConstPreview};
    use shader_editor_app::ui::node_preview::{NodePreviewsPlugin, PreviewNa, PreviewRoot};
    use shader_graph_core::Value;

    let (_dir, mut app) = app();
    app.add_plugins(NodePreviewsPlugin);
    act(&mut app, Action::OpenAsset(AssetId::new(AssetType::Material, "fx/global_tint")));
    let amount = {
        let d = app.world().resource::<Editor>().doc().unwrap();
        let (id, _) = d.asset.nodes.iter().find(|(_, n)| n.kind == "stdlib::GlobalConstF32").unwrap();
        id.clone()
    };
    // The number of the constant and whether "n/a" is visible, in the row of `amount`.
    let row = |app: &mut App| {
        let w = app.world_mut();
        let root = w.query::<(Entity, &PreviewRoot<ConstPreview>)>().iter(w).find(|(_, r)| r.node == amount).unwrap().0;
        let children: Vec<Entity> = w.get::<Children>(root).unwrap().iter().collect();
        let number = children.iter().copied().find(|c| matches!(w.get::<ConstPart>(*c), Some(ConstPart::Number)));
        let na = children.iter().copied().find(|c| w.get::<PreviewNa>(*c).is_some()).unwrap();
        let text = number.map(|n| w.get::<Text>(n).unwrap().0.clone());
        (number, text, w.get::<Visibility>(na) == Some(&Visibility::Hidden))
    };
    let (number, text, na_hidden) = row(&mut app);
    assert_eq!((text.as_deref(), na_hidden), (Some("0.35"), true));

    // A table change (here: on disk) shows on the same entities: no canvas rebuild.
    let shown = app.world().resource::<EditorCanvas>().shown.clone();
    {
        let mut e = app.world_mut().resource_mut::<Editor>();
        let table = e.disk.consts.values_mut().find_map(|t| t.as_mut().ok()).unwrap();
        table.consts.iter_mut().find(|c| c.name == "glow_amount").unwrap().value = Value::Float(0.9);
    }
    app.update();
    assert_eq!(row(&mut app), (number, Some("0.9".into()), true));
    assert_eq!(app.world().resource::<EditorCanvas>().shown, shown, "not rebuilt");

    // A node reading an unknown constant shows "n/a".
    let target = PropTarget::Param { node: amount.clone(), key: "const".into() };
    act(&mut app, Action::SetValue { target, value: Value::Str("missing".into()), merge: false });
    let (_, text, na_hidden) = row(&mut app);
    assert_eq!((text.as_deref(), na_hidden), (Some(""), false));
}

/// A preview kind whose data never draws (as an image that fails to decode).
struct FailingPreview;

impl shader_editor_app::ui::node_preview::NodePreview for FailingPreview {
    type Key = ();
    type Draw = ();

    fn applies(_: &dyn shader_graph_core::registry::NodeDef) -> bool {
        true
    }

    fn layout(_: &dyn shader_graph_core::registry::NodeDef) -> Node {
        Node::default()
    }

    fn content(_: &dyn shader_graph_core::registry::NodeDef) -> Box<dyn Scene> {
        Box::new(bsn! { Node })
    }

    fn key(_: &Editor, _: &shader_editor_app::doc::Doc, _: &String) -> Option<()> {
        Some(())
    }

    fn draw(_: Option<&()>, _: shader_editor_app::ui::node_preview::PreviewTarget, _: &mut ()) -> bool {
        false
    }
}

#[test]
fn a_preview_that_fails_to_draw_shows_na() {
    use shader_editor_app::ui::node_preview::{NodePreviewPlugin, PreviewNa, PreviewRoot, preview_row};

    let (_dir, mut app) = app();
    app.add_plugins(NodePreviewPlugin::<FailingPreview>::default());
    act(&mut app, Action::OpenAsset(AssetId::new(AssetType::Material, "fx/global_tint")));
    let def = app.world().resource::<Editor>().registry.get("stdlib::Time").unwrap().clone();
    let row = preview_row::<FailingPreview>(def.as_ref(), &"n1".to_string()).unwrap();
    app.world_mut().spawn_scene(row).unwrap();
    app.update();
    let w = app.world_mut();
    let root = w.query_filtered::<Entity, With<PreviewRoot<FailingPreview>>>().single(w).unwrap();
    let na = w.get::<Children>(root).unwrap().iter().find(|c| w.get::<PreviewNa>(*c).is_some()).unwrap();
    assert_eq!(w.get::<Visibility>(na), Some(&Visibility::Inherited), "a key that does not draw shows n/a");
}

#[test]
fn texture_thumbnail_shows_the_image_and_na_without_rebuild() {
    use shader_editor_app::ui::node_preview::{NodePreviewsPlugin, PreviewNa, PreviewRoot};
    use shader_editor_app::ui::texture_preview::{SIDE, TexturePart, TexturePreview};
    use shader_graph_core::Value;

    let (dir, mut app) = app();
    let textures = dir.path().join("assets/textures");
    std::fs::create_dir_all(&textures).unwrap();
    let sample = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/sample_game/assets/textures/rock_albedo.ktx2");
    std::fs::copy(sample, textures.join("rock_albedo.ktx2")).unwrap();
    app.add_plugins(NodePreviewsPlugin);
    act(&mut app, Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet")));
    let node = |app: &App, default: &str| {
        let d = app.world().resource::<Editor>().doc().unwrap();
        let v = Value::Str(default.into());
        d.asset.nodes.iter().find(|(_, n)| n.params.get("default") == Some(&v)).unwrap().0.clone()
    };
    let (albedo, normal) = (node(&app, "textures/rock_albedo"), node(&app, "textures/rock_normal"));
    // (row root, "n/a" visible, image tint, image width) of `node`'s thumbnail.
    let row = |app: &mut App, node: &str| {
        let w = app.world_mut();
        let root = w.query::<(Entity, &PreviewRoot<TexturePreview>)>().iter(w).find(|(_, r)| r.node == node).unwrap().0;
        let na = w.get::<Children>(root).unwrap().iter().find(|c| w.get::<PreviewNa>(*c).is_some()).unwrap();
        let image = w
            .query::<(Entity, &TexturePart)>()
            .iter(w)
            .find(|(e, p)| {
                **p == TexturePart::Image
                    && std::iter::successors(Some(*e), |e| w.get::<ChildOf>(*e).map(ChildOf::parent)).any(|e| e == root)
            })
            .unwrap()
            .0;
        let (tint, width) = (w.get::<ImageNode>(image).unwrap().color, w.get::<Node>(image).unwrap().width);
        (root, w.get::<Visibility>(na) == Some(&Visibility::Inherited), tint, width)
    };
    let (root, na, tint, width) = row(&mut app, &albedo);
    assert_eq!((na, tint, width), (false, Color::WHITE, px(SIDE)), "the image fills the square");
    let (_, na, tint, _) = row(&mut app, &normal);
    assert_eq!((na, tint), (true, Color::NONE), "rock_normal.png is not in this project: n/a");

    // The Texture asset now points at a missing file (here: on disk): n/a on the same row, no canvas rebuild.
    let shown = app.world().resource::<EditorCanvas>().shown.clone();
    {
        let mut e = app.world_mut().resource_mut::<Editor>();
        let id = AssetId::new(AssetType::Texture, "textures/rock_albedo");
        e.disk.textures.get_mut(&id).unwrap().as_mut().unwrap().path = "textures/gone.png".into();
    }
    app.update();
    let (same, na, tint, _) = row(&mut app, &albedo);
    assert_eq!((same, na, tint), (root, true, Color::NONE));
    assert_eq!(app.world().resource::<EditorCanvas>().shown, shown, "not rebuilt");
}
