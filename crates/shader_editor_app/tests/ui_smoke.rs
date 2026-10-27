//! Canvas smoke tests without a window (spec §15 "UI"): create a node, connect, undo, save —
//! through the bevy_noodle API, as the user's gestures would.

use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy_noodle::prelude::*;
use bevy_noodle::style::kit;
use bevy_noodle::{MovingEdges, PortAnchor};
use shader_editor_app::EditorCorePlugin;
use shader_editor_app::canvas::{EditorCanvas, FrameKey, NodeKey, PinKey};
use shader_editor_app::state::{Action, Actions, Editor, PropTarget};
use shader_editor_app::tab::Lang;
use shader_editor_app::ui::EditorCanvasMarker;
use shader_editor_app::ui::props::PropField;
use shader_project::{AssetId, AssetType, Project, fixtures};

fn app() -> (tempfile::TempDir, App) {
    app_with(|_| {})
}

/// [`app`] with `extra` plugins added after the editor's, before the first update.
fn app_with(extra: impl FnOnce(&mut App)) -> (tempfile::TempDir, App) {
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
    extra(&mut app);
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

/// A pointer drag event of the primary button on node `node` (bubbling to it from its body), as bevy_picking sends
/// it: the pointer moved `by` graph units since the last event and `total` since the drag started (no layout runs
/// headless: the canvas is [`wires_app`]'s); then one update.
fn node_drag(app: &mut App, node: Entity, step: WireStep, by: Vec2, total: Vec2) {
    use bevy::camera::NormalizedRenderTarget;
    use bevy::picking::backend::HitData;
    use bevy::picking::events::{PointerCancel, PointerDrag, PointerDragEnd, PointerDragStart};
    use bevy::picking::pointer::{Location, PointerButton, PointerId};
    let canvas = app.world().resource::<EditorCanvas>().canvas;
    let zoom = app.world().get::<CanvasView>(canvas).unwrap().zoom;
    let (delta, distance) = (by * zoom, total * zoom);
    let target = NormalizedRenderTarget::None { width: 800, height: 600 };
    let pointer = Pointer::new(PointerId::Mouse, Location { target, position: Vec2::ZERO });
    let (hit, button, entity) = (HitData::new(Entity::PLACEHOLDER, 0.0, None, None), PointerButton::Primary, node);
    let w = app.world_mut();
    match step {
        WireStep::Start => w.trigger(PointerDragStart { entity, pointer, button, hit }),
        WireStep::Drag => w.trigger(PointerDrag { entity, pointer, button, distance, delta }),
        WireStep::End => w.trigger(PointerDragEnd { entity, pointer, button, distance }),
        WireStep::Cancel => w.trigger(PointerCancel { entity, pointer, hit }),
    }
    w.flush();
    app.update();
}

fn node_pos(app: &App, e: Entity) -> Vec2 {
    app.world().get::<NodePosition>(e).unwrap().0
}

/// Design §4.1: a pointer drag snaps the grabbed node to the 10-unit grid at every step (visibly, not on release);
/// the other selected nodes follow by the same delta.
#[test]
fn dragging_snaps_lead_node_to_grid() {
    let (_dir, mut app) = wires_app();
    let canvas = app.world().resource::<EditorCanvas>().canvas;
    let mut ents: Vec<(Entity, String)> = {
        let w = app.world_mut();
        w.query::<(Entity, &NodeKey)>().iter(w).map(|(e, k)| (e, k.0.clone())).collect()
    };
    ents.sort_by(|a, b| a.1.cmp(&b.1));
    let (lead, other) = (ents[0].0, ents[1].0);
    // Off-grid start, like a node of an older file.
    app.world_mut().get_mut::<NodePosition>(lead).unwrap().0 = Vec2::new(12.3, 7.6);
    app.world_mut().select(canvas, vec![lead, other], SelectMode::Replace);
    app.update();
    let rel = node_pos(&app, other) - node_pos(&app, lead);
    node_drag(&mut app, lead, WireStep::Start, Vec2::ZERO, Vec2::ZERO);
    node_drag(&mut app, lead, WireStep::Drag, Vec2::new(1.0, 0.5), Vec2::new(1.0, 0.5));
    assert_eq!(node_pos(&app, lead), Vec2::new(10.0, 10.0), "snaps on the first step");
    node_drag(&mut app, lead, WireStep::Drag, Vec2::new(1.0, 0.5), Vec2::new(2.0, 1.0));
    assert_eq!(node_pos(&app, lead), Vec2::new(10.0, 10.0), "stays until the next grid point is nearer");
    node_drag(&mut app, lead, WireStep::Drag, Vec2::new(20.0, -30.0), Vec2::new(22.0, -29.0));
    assert_eq!(node_pos(&app, lead), Vec2::new(30.0, -20.0));
    node_drag(&mut app, lead, WireStep::End, Vec2::ZERO, Vec2::new(22.0, -29.0));
    assert!((node_pos(&app, other) - node_pos(&app, lead) - rel).length() < 1e-3, "relative layout kept");
    for _ in 0..4 {
        app.update();
    }
    let d = app.world().resource::<Editor>().doc().unwrap();
    assert_eq!(d.asset.nodes[&ents[0].1].pos, (30.0, -20.0));
}

/// A drag that never ends (a cancelled touch may send no final step) must not leak its start into the next gesture
/// of the same node: each pointer drag start, or a pointer cancel, begins a fresh snap drag.
#[test]
fn snap_drag_restarts_per_gesture() {
    let (_dir, mut app) = wires_app();
    let lead = {
        let w = app.world_mut();
        w.query_filtered::<Entity, With<NodeKey>>().iter(w).next().unwrap()
    };
    app.world_mut().get_mut::<NodePosition>(lead).unwrap().0 = Vec2::new(12.3, 7.6);
    let pos = |app: &App| node_pos(app, lead);
    // An abandoned drag: the lead sits at (10, 10), no final step.
    node_drag(&mut app, lead, WireStep::Start, Vec2::ZERO, Vec2::ZERO);
    node_drag(&mut app, lead, WireStep::Drag, Vec2::new(1.0, 0.5), Vec2::new(1.0, 0.5));
    assert_eq!(pos(&app), Vec2::new(10.0, 10.0));
    // The next gesture starts from where the node is now, not from the old start (12.3, 7.6) → (30, 10).
    node_drag(&mut app, lead, WireStep::Start, Vec2::ZERO, Vec2::ZERO);
    node_drag(&mut app, lead, WireStep::Drag, Vec2::new(14.0, 0.0), Vec2::new(14.0, 0.0));
    assert_eq!(pos(&app), Vec2::new(20.0, 10.0), "a new gesture snaps from the current position");
    // A cancelled one leaves nothing behind either (else (20, 10) → (20, 20) measured from (10, 10)).
    node_drag(&mut app, lead, WireStep::Cancel, Vec2::ZERO, Vec2::ZERO);
    node_drag(&mut app, lead, WireStep::Start, Vec2::ZERO, Vec2::ZERO);
    node_drag(&mut app, lead, WireStep::Drag, Vec2::new(0.0, 10.0), Vec2::new(0.0, 10.0));
    assert_eq!(pos(&app), Vec2::new(20.0, 20.0));
}

/// [`app`] with bevy_noodle's default style, whose `highlight_ports` draws the pin hover.
fn styled_app() -> (tempfile::TempDir, App) {
    app_with(|app| {
        app.init_asset::<bevy::shader::Shader>().add_plugins(NoodleDefaultStylePlugin);
    })
}

/// The mouse over `port` or not, as bevy_picking keeps it in `Hovered` (also while a wire is dragged); then one
/// update.
fn hover(app: &mut App, port: Entity, over: bool) {
    app.world_mut().entity_mut(port).insert(Hovered(over));
    app.update();
}

/// A port's drawn dot (a `PortDot` child), or the port itself.
fn dot_of(app: &App, port: Entity) -> Entity {
    let w = app.world();
    let children = w.get::<Children>(port).map(|c| c.to_vec()).unwrap_or_default();
    children.into_iter().find(|c| w.get::<PortDot>(*c).is_some()).unwrap_or(port)
}

/// A port's drawn fill and scale (its dot's), as bevy_noodle's style left them.
fn port_look(app: &App, port: Entity) -> (Color, f32) {
    let (w, dot) = (app.world(), dot_of(app, port));
    (w.get::<BackgroundColor>(dot).unwrap().0, w.get::<UiTransform>(dot).unwrap().scale.x)
}

/// Design §4.8: a hovered pin's dot is shown 1.3x its size and lighter (bevy_noodle's `PortHoverStyle`).
#[test]
fn pin_hover_highlights_the_dot() {
    let (_dir, mut app) = styled_app();
    act(&mut app, Action::OpenAsset(AssetId::new(AssetType::Material, "fx/hologram")));
    let pin = {
        let w = app.world_mut();
        w.query_filtered::<Entity, With<PinKey>>().iter(w).next().unwrap()
    };
    let (fill, scale) = port_look(&app, pin);
    assert_eq!(scale, 1.0);
    hover(&mut app, pin, true);
    let (hover_fill, hover_scale) = port_look(&app, pin);
    assert!((hover_scale - 1.3).abs() < 1e-6, "{hover_scale}");
    assert!(hover_fill.luminance() > fill.luminance(), "lighter than the dot");
    // Design §6.3: only the drawn dot grows; the pin's physical box, its hit area, keeps its size.
    assert_ne!(dot_of(&app, pin), pin);
    assert_eq!(app.world().get::<UiTransform>(pin).unwrap().scale, Vec2::ONE, "the hit area is unchanged");
    hover(&mut app, pin, false);
    assert_eq!(port_look(&app, pin), (fill, scale));
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

    // A focused widget that takes no text (here the frame of the alpha field) must not freeze the canvas.
    let widget = inline_entity(&mut app, &lerp, "alpha").unwrap();
    focus(&mut app, widget);
    let alpha = PropTarget::Input { node: lerp.clone(), pin: "alpha".into() };
    act(&mut app, Action::SetComponent { target: alpha, comp: 0, value: 0.7, is_final: true });
    assert!(app.world().get_entity(widget).is_err(), "rebuilt despite the focused widget");

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

/// The per-kind strips (decision 0031): only the active kind's `AreaOf` containers are displayed; with no tab the
/// editor topbar strip is not displayed at all.
#[test]
fn areas_show_only_active_kind() {
    use shader_editor_app::editors::AreaOf;
    use shader_editor_app::tab::ViewKind;
    use shader_editor_app::ui::areas::{AreasPlugin, EditorTopbar, editor_topbar, right_sidebar};

    let (_dir, mut app) = app();
    app.add_plugins(AreasPlugin);
    app.world_mut().spawn_scene(editor_topbar()).unwrap();
    app.world_mut().spawn_scene(right_sidebar()).unwrap();
    act(&mut app, Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet")));
    let areas = |app: &mut App| {
        let w = app.world_mut();
        w.query::<(&AreaOf, &Node)>().iter(w).map(|(a, n)| (a.0, n.display)).collect::<Vec<_>>()
    };
    let shown = areas(&mut app);
    assert!(shown.iter().filter(|(k, _)| *k == ViewKind::Graph).count() >= 2, "graph topbar and sidebar: {shown:?}");
    for (kind, display) in &shown {
        let want = if *kind == ViewKind::Graph { Display::Flex } else { Display::None };
        assert_eq!(*display, want, "{kind:?}");
    }
    let topbar = |app: &mut App| {
        let w = app.world_mut();
        w.query_filtered::<&Node, With<EditorTopbar>>().single(w).unwrap().display
    };
    assert_eq!(topbar(&mut app), Display::Flex);

    act(&mut app, Action::CloseTab(usize::MAX));
    assert!(app.world().resource::<Editor>().tab().is_none());
    assert_eq!(topbar(&mut app), Display::None, "no tab: no editor topbar");
    assert!(areas(&mut app).iter().all(|(_, d)| *d == Display::None));
}

/// The pin under the pointer keeps its hover look when the canvas is rebuilt from the model (its port entity is
/// replaced, and picking sees the new one only a frame later).
#[test]
fn pin_hover_survives_rebuild() {
    let (_dir, mut app) = styled_app();
    act(&mut app, Action::OpenAsset(AssetId::new(AssetType::Material, "fx/hologram")));
    let (pin, key) = {
        let w = app.world_mut();
        w.query::<(Entity, &PinKey)>().iter(w).map(|(e, k)| (e, k.clone())).next().unwrap()
    };
    hover(&mut app, pin, true);
    let shown_on = |app: &mut App, key: &PinKey| {
        let w = app.world_mut();
        let port = w.query::<(Entity, &PinKey)>().iter(w).find(|(_, k)| *k == key).unwrap().0;
        (port, w.get::<Hovered>(port).is_some_and(|h| h.0), port_look(app, port).1)
    };
    assert_eq!(shown_on(&mut app, &key), (pin, true, 1.3));

    let revision = app.world().resource::<Editor>().doc().unwrap().revision;
    act(&mut app, Action::AddNode { kind: "stdlib::Time".into(), pos: (900.0, 900.0), link: None, params: Vec::new() });
    assert!(app.world().resource::<Editor>().doc().unwrap().revision > revision);
    let (port, hovered, scale) = shown_on(&mut app, &key);
    assert_ne!(port, pin, "the canvas was rebuilt");
    assert!(hovered && (scale - 1.3).abs() < 1e-6, "the hovered pin keeps its look: {hovered} {scale}");
}

/// The rock_wet comment frame `c01` and its canvas entity.
fn frame(app: &mut App) -> (Entity, (f32, f32, f32, f32)) {
    let rect = {
        let d = app.world().resource::<Editor>().doc().unwrap();
        d.asset.comments.iter().find(|f| f.id == "c01").unwrap().rect
    };
    let w = app.world_mut();
    let e = w.query::<(Entity, &FrameKey)>().iter(w).find(|(_, k)| k.0 == "c01").unwrap().0;
    (e, rect)
}

/// A frame dragged alone is the lead (decision 0033): its origin snaps to the grid.
#[test]
fn frame_drag_snaps_origin() {
    let (_dir, mut app) = wires_app();
    let (f, _) = frame(&mut app);
    app.world_mut().get_mut::<NodePosition>(f).unwrap().0 = Vec2::new(2.0, 3.0);
    let pos = |app: &App| node_pos(app, f);
    node_drag(&mut app, f, WireStep::Start, Vec2::ZERO, Vec2::ZERO);
    node_drag(&mut app, f, WireStep::Drag, Vec2::new(4.0, 2.5), Vec2::new(4.0, 2.5));
    assert_eq!(pos(&app), Vec2::new(10.0, 10.0));
    node_drag(&mut app, f, WireStep::Drag, Vec2::new(12.0, 5.5), Vec2::new(16.0, 8.0));
    node_drag(&mut app, f, WireStep::End, Vec2::ZERO, Vec2::new(16.0, 8.0));
    assert_eq!(pos(&app), Vec2::new(20.0, 10.0));
    for _ in 0..4 {
        app.update();
    }
    let (_, rect) = frame(&mut app);
    assert_eq!((rect.0, rect.1), (20.0, 10.0), "the snapped origin is stored");
}

/// The nodes a dragged frame carries take the frame's snapped delta and are not snapped again: an off-grid node
/// inside keeps its offset from the grid; the carry is part of the frame's drag and lands in the model with it.
#[test]
fn carried_nodes_not_resnapped() {
    let (_dir, mut app) = wires_app();
    let (f, _) = frame(&mut app);
    let (node, id) = {
        let w = app.world_mut();
        w.query::<(Entity, &NodeKey)>().iter(w).map(|(e, k)| (e, k.0.clone())).next().unwrap()
    };
    // No UI layout headless: give the frame and the node a size so the frame encloses the node.
    let sized = |size: Vec2| ComputedNode { size, inverse_scale_factor: 1.0, ..default() };
    let w = app.world_mut();
    w.entity_mut(f).insert((NodePosition(Vec2::new(2.0, 3.0)), sized(Vec2::new(400.0, 300.0))));
    w.entity_mut(node).insert((NodePosition(Vec2::new(12.3, 7.6)), sized(Vec2::new(100.0, 50.0))));
    let close = |a: Vec2, b: Vec2| (a - b).length() < 1e-4;

    node_drag(&mut app, f, WireStep::Start, Vec2::ZERO, Vec2::ZERO);
    node_drag(&mut app, f, WireStep::Drag, Vec2::new(4.0, 2.5), Vec2::new(4.0, 2.5));
    assert_eq!(node_pos(&app, f), Vec2::new(10.0, 10.0));
    assert!(
        close(node_pos(&app, node), Vec2::new(20.3, 14.6)),
        "carried by the frame's delta: {}",
        node_pos(&app, node)
    );
    // The next step still measures from the frame's start (else (30, 20)).
    node_drag(&mut app, f, WireStep::Drag, Vec2::new(12.0, 5.5), Vec2::new(16.0, 8.0));
    assert_eq!(node_pos(&app, f), Vec2::new(20.0, 10.0));
    assert!(close(node_pos(&app, node), Vec2::new(30.3, 14.6)), "{}", node_pos(&app, node));
    node_drag(&mut app, f, WireStep::End, Vec2::ZERO, Vec2::new(16.0, 8.0));
    for _ in 0..4 {
        app.update();
    }
    let d = app.world().resource::<Editor>().doc().unwrap();
    let (x, y) = d.asset.nodes[&id].pos;
    assert!(close(Vec2::new(x, y), Vec2::new(30.3, 14.6)), "stored with the frame's move: {x}, {y}");
}

/// A selection box marks a comment frame for Delete only when it fully encloses it (decision 0033); bevy_noodle
/// selects every frame the box touches.
#[test]
fn box_select_marks_enclosed_frame_only_when_fully_inside() {
    use bevy_noodle::prelude::SelectionBox;

    let (_dir, mut app) = app();
    act(&mut app, Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet")));
    let canvas = app.world().resource::<EditorCanvas>().canvas;
    let enclosed = |app: &App| app.world().resource::<Editor>().doc().unwrap().enclosed_frames.clone();
    // A box from `from` to `to` in graph space, selecting the frame and one node as bevy_noodle would.
    let boxed = |app: &mut App, from: Vec2, to: Vec2| {
        let (f, _) = frame(app);
        let node = {
            let w = app.world_mut();
            w.query_filtered::<Entity, With<NodeKey>>().iter(w).next().unwrap()
        };
        let view = *app.world().get::<CanvasView>(canvas).unwrap();
        let area = Rect::from_corners(view.graph_to_canvas(from), view.graph_to_canvas(to));
        app.world_mut().entity_mut(canvas).insert(SelectionBox(area));
        app.world_mut().select(canvas, vec![node, f], SelectMode::Replace);
        app.update();
        app.update();
        app.update();
        app.world_mut().entity_mut(canvas).remove::<SelectionBox>();
        app.update();
    };
    let (_, (x, y, w, h)) = frame(&mut app);
    let (min, max) = (Vec2::new(x, y), Vec2::new(x + w, y + h));

    boxed(&mut app, min - 10.0, min + (max - min) / 2.0);
    assert!(enclosed(&app).is_empty(), "a box ending in the frame's middle");
    app.world_mut().select(canvas, vec![], SelectMode::Replace);
    app.update();
    app.update();
    boxed(&mut app, min - 10.0, max + 10.0);
    assert_eq!(enclosed(&app), ["c01".to_string()].into(), "a box around the whole frame");
}

/// The editor statusbar's `● unsaved` follows the document (its key holds no dirty flag, see `ui::areas`): an edit
/// shows it, an undo back to the saved state hides it, an edit and a save hide it again.
#[test]
fn statusbar_dirty_mark_follows_edit_undo_save() {
    use shader_editor_app::ui::areas::{AreasPlugin, EditorStatusbar, editor_statusbar};

    let (_dir, mut app) = app();
    app.add_plugins(AreasPlugin);
    app.world_mut().spawn_scene(editor_statusbar()).unwrap();
    act(&mut app, Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet")));
    let unsaved = |app: &mut App| {
        let w = app.world_mut();
        let strip = w.query_filtered::<Entity, With<EditorStatusbar>>().single(w).unwrap();
        let texts: Vec<String> = w
            .query::<(Entity, &Text)>()
            .iter(w)
            .filter_map(|(e, t)| {
                let mut at = e;
                while let Some(p) = w.get::<ChildOf>(at) {
                    at = p.parent();
                    if at == strip {
                        return Some(t.0.clone());
                    }
                }
                None
            })
            .collect();
        assert!(!texts.is_empty(), "the statusbar shows something");
        texts.iter().any(|t| t.contains("unsaved"))
    };
    assert!(!unsaved(&mut app));
    let time = Action::AddNode { kind: "stdlib::Time".into(), pos: (900.0, 900.0), link: None, params: Vec::new() };
    act(&mut app, time.clone());
    assert!(unsaved(&mut app), "after an edit");
    act(&mut app, Action::Undo);
    assert!(!unsaved(&mut app), "undone back to the saved state");
    act(&mut app, time);
    assert!(unsaved(&mut app));
    act(&mut app, Action::Save);
    assert!(!unsaved(&mut app), "saved");
}

/// A full pointer click on `entity` in bevy_picking's order (press, click, release); the events bubble up to the
/// widget the entity belongs to.
fn click(app: &mut App, entity: Entity) {
    use std::time::Duration;

    use bevy::camera::NormalizedRenderTarget;
    use bevy::picking::backend::HitData;
    use bevy::picking::events::{PointerClick, PointerPress, PointerRelease};
    use bevy::picking::pointer::{Location, PointerButton, PointerId};
    let target = NormalizedRenderTarget::None { width: 800, height: 600 };
    let pointer = Pointer::new(PointerId::Mouse, Location { target, position: Vec2::ZERO });
    let hit = HitData::new(Entity::PLACEHOLDER, 0.0, None, None);
    let button = PointerButton::Primary;
    let w = app.world_mut();
    // Pointer events bubble through `PointerTraversal`, which reads `Window`: unregistered (no window here), the
    // traversal fails and the event stops at `entity`.
    w.register_component::<Window>();
    // Each step sees the previous one's commands (the widget's `Pressed`), as between picking's frames.
    w.trigger(PointerPress { entity, pointer: pointer.clone(), button, hit: hit.clone(), count: 1 });
    w.flush();
    w.trigger(PointerClick {
        entity,
        pointer: pointer.clone(),
        button,
        hit: hit.clone(),
        duration: Duration::from_millis(50),
        count: 1,
    });
    w.flush();
    w.trigger(PointerRelease { entity, pointer, button, hit });
    w.flush();
}

/// A primary double click on `entity` (its second click), bubbling up to the node or frame it belongs to; the
/// actions it pushes are applied.
fn dblclick(app: &mut App, entity: Entity) {
    use bevy::camera::NormalizedRenderTarget;
    use bevy::picking::backend::HitData;
    use bevy::picking::events::PointerClick;
    use bevy::picking::pointer::{Location, PointerButton, PointerId};
    let target = NormalizedRenderTarget::None { width: 800, height: 600 };
    let pointer = Pointer::new(PointerId::Mouse, Location { target, position: Vec2::ZERO });
    let hit = HitData::new(Entity::PLACEHOLDER, 0.0, None, None);
    let w = app.world_mut();
    w.register_component::<Window>();
    w.trigger(PointerClick {
        entity,
        pointer,
        button: PointerButton::Primary,
        hit,
        duration: std::time::Duration::from_millis(50),
        count: 2,
    });
    w.flush();
    app.update();
    app.update();
}

/// [`app`] with the canvas gestures (`ui::input`).
fn gestures_app() -> (tempfile::TempDir, App) {
    app_with(|app| {
        app.add_message::<bevy::window::WindowCloseRequested>()
            .init_resource::<UiScale>()
            .init_resource::<bevy::picking::hover::HoverMap>()
            .init_resource::<shader_editor_app::ui::palette::Palette>()
            .add_plugins(shader_editor_app::ui::input::InputPlugin);
    })
}

/// The canvas entity of the first node of `kind`, and its header (title bar).
fn node_and_header(app: &mut App, kind: &str) -> (Entity, Entity) {
    use shader_editor_app::canvas::NodeHeader;
    let id = {
        let d = app.world().resource::<Editor>().doc().unwrap();
        d.asset.nodes.iter().find(|(_, n)| n.kind == kind).unwrap_or_else(|| panic!("no {kind}")).0.clone()
    };
    let w = app.world_mut();
    let node = w.query::<(Entity, &NodeKey)>().iter(w).find(|(_, k)| k.0 == id).unwrap().0;
    let header = w
        .query_filtered::<(Entity, &ChildOf), With<NodeHeader>>()
        .iter(w)
        .find(|(_, c)| c.parent() == node)
        .unwrap_or_else(|| panic!("{kind} has no header"))
        .0;
    (node, header)
}

fn active_tab(app: &App) -> Option<AssetId> {
    let e = app.world().resource::<Editor>();
    e.active.and_then(|i| e.tabs[i].owner().cloned())
}

/// Design §5.5: a double click on a node header renames the node if it has a name or label, else opens the file it
/// refers to, else does nothing; on the body it opens the file as before.
#[test]
fn header_dblclick_opens_rename_or_reference() {
    use shader_editor_app::state::{Dialog, Submit};

    let (_dir, mut app) = gestures_app();
    let material = AssetId::new(AssetType::Material, "env/rock_wet");
    let function = AssetId::new(AssetType::Function, "common/wet_mask");
    act(&mut app, Action::OpenAsset(material.clone()));

    let (_, header) = node_and_header(&mut app, "param::Scalar");
    dblclick(&mut app, header);
    let dialog = app.world().resource::<Editor>().dialog.clone();
    let renames = |t: &PropTarget| matches!(t, PropTarget::Param { key, .. } if key == "name");
    assert!(matches!(&dialog, Some(Dialog::Input { submit: Submit::SetText(t), .. }) if renames(t)), "{dialog:?}");
    act(&mut app, Action::CloseDialog);

    let (_, header) = node_and_header(&mut app, "stdlib::Lerp");
    dblclick(&mut app, header);
    assert_eq!(app.world().resource::<Editor>().dialog, None, "Lerp: nothing");
    assert_eq!(active_tab(&app), Some(material.clone()));

    let (_, header) = node_and_header(&mut app, "function::common/wet_mask");
    dblclick(&mut app, header);
    assert_eq!(active_tab(&app), Some(function.clone()), "a function call's header opens the function");
    assert_eq!(app.world().resource::<Editor>().dialog, None);

    act(&mut app, Action::OpenAsset(material.clone()));
    let (node, _) = node_and_header(&mut app, "function::common/wet_mask");
    dblclick(&mut app, node);
    assert_eq!(active_tab(&app), Some(function), "the body opens it too");

    // The body of a named node does not rename it.
    act(&mut app, Action::OpenAsset(material));
    let (node, _) = node_and_header(&mut app, "param::Scalar");
    dblclick(&mut app, node);
    assert_eq!(app.world().resource::<Editor>().dialog, None, "a param's body");
}

/// Design §5.5: a double click on a comment frame's title strip edits the title; on the rest of the frame it does
/// nothing.
#[test]
fn frame_title_dblclick_opens_dialog() {
    use shader_editor_app::canvas::FrameTitleStrip;
    use shader_editor_app::state::{Dialog, Submit};

    let (_dir, mut app) = gestures_app();
    act(&mut app, Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet")));
    let (f, _) = frame(&mut app);
    dblclick(&mut app, f);
    assert_eq!(app.world().resource::<Editor>().dialog, None, "the frame's body");
    let strip = {
        let w = app.world_mut();
        w.query_filtered::<(Entity, &ChildOf), With<FrameTitleStrip>>()
            .iter(w)
            .find(|(_, c)| c.parent() == f)
            .expect("a title strip")
            .0
    };
    dblclick(&mut app, strip);
    let dialog = app.world().resource::<Editor>().dialog.clone();
    let title = {
        let d = app.world().resource::<Editor>().doc().unwrap();
        d.asset.comments.iter().find(|c| c.id == "c01").unwrap().title.clone()
    };
    match &dialog {
        Some(Dialog::Input { title: t, label, initial, submit: Submit::SetText(target) }) => {
            assert_eq!((t.as_str(), label.as_str()), ("Comment", "Title"));
            assert_eq!(*initial, title);
            assert_eq!(*target, PropTarget::FrameTitle("c01".into()));
        }
        other => panic!("{other:?}"),
    }
    act(&mut app, Action::Submit("Wet rocks".into()));
    let d = app.world().resource::<Editor>().doc().unwrap();
    assert_eq!(d.asset.comments.iter().find(|c| c.id == "c01").unwrap().title, "Wet rocks");
}

/// The entity showing `label` (a `Text`), inside the subtree of `root`.
fn text_entity(app: &mut App, root: Entity, label: &str) -> Option<Entity> {
    let w = app.world_mut();
    let found: Vec<Entity> =
        w.query::<(Entity, &Text)>().iter(w).filter(|(_, t)| t.0 == label).map(|(e, _)| e).collect();
    found
        .into_iter()
        .find(|e| std::iter::successors(Some(*e), |e| w.get::<ChildOf>(*e).map(ChildOf::parent)).any(|e| e == root))
}

/// Design §3: a checkbox carries its label as its Feathers caption, so clicking the label toggles it — the texture
/// tab's "Tile 3x3" in the editor topbar.
#[test]
fn checkbox_label_click_toggles() {
    use shader_editor_app::game_image::GameImagePlugin;
    use shader_editor_app::ui::areas::{AreasPlugin, editor_topbar};
    use shader_editor_app::ui::image_preview::ImagePreviewPlugin;
    use shader_editor_app::ui::image_view::ImageViewPlugin;
    use shader_editor_app::ui::props::PropsPlugin;
    use shader_editor_app::ui::texture_view::{TextureViewPlugin, texture_view};

    let (_dir, mut app) = app();
    app.add_plugins((
        AreasPlugin,
        PropsPlugin,
        GameImagePlugin,
        ImagePreviewPlugin,
        ImageViewPlugin,
        TextureViewPlugin,
        bevy::ui_widgets::CheckboxPlugin,
    ));
    let bar = app.world_mut().spawn_scene(editor_topbar()).unwrap().id();
    app.world_mut().spawn_scene(texture_view()).unwrap();
    act(&mut app, Action::OpenAsset(AssetId::new(AssetType::Texture, "textures/rock_albedo")));
    let tiled = |app: &App| app.world().resource::<Editor>().texture_doc().unwrap().tiled;
    assert!(!tiled(&app));
    let label = text_entity(&mut app, bar, "Tile 3x3").expect("the editor topbar shows Tile 3x3");
    click(&mut app, label);
    app.update();
    app.update();
    assert!(tiled(&app), "a click on the label toggles the checkbox");
    let label = text_entity(&mut app, bar, "Tile 3x3").expect("still shown after the rebuild");
    click(&mut app, label);
    app.update();
    app.update();
    assert!(!tiled(&app), "and back");
}

/// Design §6.2: the image tab has the texture tab's preview — the wheel zooms, a double click fits, and each tab
/// keeps its own zoom.
#[test]
fn image_tab_zooms_and_fits() {
    use bevy::input::mouse::MouseScrollUnit;
    use bevy::input::touch::TouchPhase;
    use bevy::picking::backend::HitData;
    use bevy::picking::events::{PointerClick, PointerScroll};
    use bevy::picking::pointer::{Location, PointerButton, PointerId};
    use shader_editor_app::game_image::GameImagePlugin;
    use shader_editor_app::tab::ViewKind;
    use shader_editor_app::ui::areas::{AreasPlugin, editor_topbar};
    use shader_editor_app::ui::image_preview::{ImagePreviewPlugin, PreviewArea, PreviewZoomText};
    use shader_editor_app::ui::image_view::{ImageViewPlugin, image_view};

    let (dir, mut app) = app();
    let textures = dir.path().join("assets/textures");
    std::fs::create_dir_all(&textures).unwrap();
    let samples = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/sample_game/assets/textures");
    for f in ["sprite.png", "atlas.png"] {
        std::fs::copy(format!("{samples}/{f}"), textures.join(f)).unwrap();
    }
    app.add_plugins((AreasPlugin, GameImagePlugin, ImagePreviewPlugin, ImageViewPlugin));
    app.world_mut().spawn_scene(editor_topbar()).unwrap();
    app.world_mut().spawn_scene(image_view()).unwrap();
    act(&mut app, Action::OpenGameImage("textures/sprite.png".into()));
    let zoom = |app: &mut App| {
        let w = app.world_mut();
        w.query_filtered::<&Text, With<PreviewZoomText>>().iter(w).map(|t| t.0.clone()).collect::<Vec<_>>()
    };
    assert_eq!(zoom(&mut app), vec!["Fit".to_string()]);
    let area = {
        let w = app.world_mut();
        w.query::<(Entity, &PreviewArea)>().iter(w).find(|(_, a)| a.0 == ViewKind::Image).expect("image preview").0
    };
    let target = bevy::camera::NormalizedRenderTarget::None { width: 800, height: 600 };
    let pointer = Pointer::new(PointerId::Mouse, Location { target, position: Vec2::new(300.0, 200.0) });
    let hit = HitData::new(Entity::PLACEHOLDER, 0.0, None, None);
    let w = app.world_mut();
    w.register_component::<Window>();
    for _ in 0..3 {
        w.trigger(PointerScroll {
            entity: area,
            pointer: pointer.clone(),
            unit: MouseScrollUnit::Line,
            x: 0.0,
            y: 1.0,
            hit: hit.clone(),
            phase: TouchPhase::Moved,
        });
        w.flush();
    }
    app.update();
    let zoomed = zoom(&mut app);
    assert!(zoomed.len() == 1 && zoomed[0].ends_with('%'), "the wheel zooms: {zoomed:?}");

    // Another image fits; back on the first, its zoom is kept.
    act(&mut app, Action::OpenGameImage("textures/atlas.png".into()));
    assert_eq!(zoom(&mut app), vec!["Fit".to_string()], "each tab has its own zoom");
    act(&mut app, Action::ActivateTab(0));
    assert_eq!(zoom(&mut app), zoomed, "kept per tab");

    let w = app.world_mut();
    w.trigger(PointerClick {
        entity: area,
        pointer,
        button: PointerButton::Primary,
        hit,
        duration: std::time::Duration::from_millis(50),
        count: 2,
    });
    w.flush();
    app.update();
    assert_eq!(zoom(&mut app), vec!["Fit".to_string()], "a double click fits");
}

/// Design §6.3: with a sampler that matches no preset the editor topbar's Easy button is disabled and says why; it
/// follows a dragged number field (which keeps the form) too.
#[test]
fn easy_button_disabled_for_custom_sampler() {
    use bevy::ui::InteractionDisabled;
    use shader_editor_app::game_image::GameImagePlugin;
    use shader_editor_app::state::TextureAction;
    use shader_editor_app::texture_doc::TextureField;
    use shader_editor_app::ui::OnActivate;
    use shader_editor_app::ui::areas::{AreasPlugin, editor_topbar};
    use shader_editor_app::ui::image_preview::ImagePreviewPlugin;
    use shader_editor_app::ui::texture_view::{TextureViewPlugin, texture_view};
    use shader_editor_app::ui::tooltip::Tooltip;
    use shader_graph_core::Value;

    let (_dir, mut app) = app();
    app.add_plugins((AreasPlugin, GameImagePlugin, ImagePreviewPlugin, TextureViewPlugin));
    app.world_mut().spawn_scene(editor_topbar()).unwrap();
    app.world_mut().spawn_scene(texture_view()).unwrap();
    act(&mut app, Action::OpenAsset(AssetId::new(AssetType::Texture, "textures/sprite")));
    act(&mut app, Action::Texture(TextureAction::SetEasy(false)));
    let easy = |app: &mut App| {
        let w = app.world_mut();
        let mut q = w.query::<(Entity, &OnActivate, &Tooltip)>();
        let (e, _, tip) = q
            .iter(w)
            .find(|(_, a, _)| matches!(a.0, Action::Texture(TextureAction::SetEasy(true))))
            .expect("the Easy button");
        (w.get::<InteractionDisabled>(e).is_some(), tip.0.clone())
    };
    assert!(!easy(&mut app).0, "Default fits Easy");
    // A dragged field: one merged step, the form is kept, the topbar follows.
    let target = PropTarget::Texture(TextureField::LodMaxClamp);
    act(&mut app, Action::SetComponent { target, comp: 0, value: 4.0, is_final: false });
    let (disabled, tip) = easy(&mut app);
    assert!(disabled, "a custom sampler disables Easy");
    assert_eq!(tip, "Settings match no preset - edit them in Advanced");
    act(
        &mut app,
        Action::SetValue {
            target: PropTarget::Texture(TextureField::DefaultSampler),
            value: Value::Bool(true),
            merge: false,
        },
    );
    assert!(!easy(&mut app).0, "back on Default");
}

/// A disabled `GroupButton` swallows the click (`InteractionDisabled`: no `Activate`, no action); an enabled one of
/// the same group pushes its action.
#[test]
fn button_group_disabled_swallows_activate() {
    use shader_editor_app::ui::OnActivate;
    use shader_editor_app::ui::widgets::{Caption, GroupButton, button_group};

    let (_dir, mut app) = app();
    app.add_plugins(bevy::ui_widgets::ButtonPlugin).add_observer(shader_editor_app::ui::activate_action);
    let item = |label: &str, action: Action, disabled: bool| GroupButton {
        caption: Caption::Text(label.into()),
        action,
        selected: false,
        disabled,
        tip: label.into(),
    };
    let group = vec![item("On", Action::ClearMessages, false), item("Off", Action::RegenAll, true)];
    app.world_mut().spawn_scene(button_group(group)).unwrap();
    app.update();
    let button = |app: &mut App, want: fn(&Action) -> bool| {
        let w = app.world_mut();
        w.query::<(Entity, &OnActivate)>().iter(w).find(|(_, a)| want(&a.0)).map(|(e, _)| e).unwrap()
    };
    let pushed = |app: &mut App| std::mem::take(&mut app.world_mut().resource_mut::<Actions>().0);
    let off = button(&mut app, |a| matches!(a, Action::RegenAll));
    click(&mut app, off);
    assert!(pushed(&mut app).is_empty(), "a disabled button pushes nothing");
    let on = button(&mut app, |a| matches!(a, Action::ClearMessages));
    click(&mut app, on);
    assert!(matches!(pushed(&mut app)[..], [Action::ClearMessages]), "an enabled one pushes its action");
}

/// The graph editor topbar's buttons (`OnActivate`) whose action `pick` accepts, with their `InteractionDisabled`,
/// `ButtonVariant` and the tints of their icons.
fn topbar_buttons<T>(
    app: &mut App,
    pick: impl Fn(&Action) -> Option<T>,
) -> Vec<(T, bool, bevy::feathers::controls::ButtonVariant, Vec<Color>)> {
    use bevy::feathers::controls::ButtonVariant;
    use bevy::ui::InteractionDisabled;
    use shader_editor_app::ui::OnActivate;
    use shader_editor_app::ui::icons::UiIcon;

    let w = app.world_mut();
    // One per icon: an icon's accent layer is a `UiIcon` child of it.
    let icons: Vec<(Entity, Color)> = w
        .query::<(Entity, &ImageNode, &UiIcon, Option<&ChildOf>)>()
        .iter(w)
        .filter(|(.., parent)| parent.is_none_or(|p| w.get::<UiIcon>(p.parent()).is_none()))
        .map(|(e, img, ..)| (e, img.color))
        .collect();
    let mut q = w.query::<(Entity, &OnActivate, Option<&ButtonVariant>)>();
    let found: Vec<(Entity, T, ButtonVariant)> =
        q.iter(w).filter_map(|(e, a, v)| Some((e, pick(&a.0)?, v.cloned().unwrap_or_default()))).collect();
    found
        .into_iter()
        .map(|(e, t, v)| {
            let inside = |icon: Entity| {
                std::iter::successors(Some(icon), |x| w.get::<ChildOf>(*x).map(ChildOf::parent)).any(|x| x == e)
            };
            let tints = icons.iter().filter(|(i, _)| inside(*i)).map(|(_, c)| *c).collect();
            (t, w.get::<InteractionDisabled>(e).is_some(), v, tints)
        })
        .collect()
}

/// Design §5.2: the graph editor topbar has icon-only "Show generated" buttons — WESL for materials and functions,
/// Rust for materials only — refilled when another graph tab becomes active.
#[test]
fn graph_topbar_buttons_per_kind() {
    use shader_editor_app::ui::areas::{AreasPlugin, editor_topbar};
    use shader_editor_app::ui::tooltip::Tooltip;

    let (_dir, mut app) = app();
    app.add_plugins(AreasPlugin);
    app.world_mut().spawn_scene(editor_topbar()).unwrap();
    let generated = |app: &mut App| {
        let mut found = topbar_buttons(app, |a| match a {
            Action::ShowGenerated(id, lang) => Some((id.source_rel(), *lang)),
            _ => None,
        });
        found.sort_by_key(|b| format!("{:?}", b.0));
        found.into_iter().map(|b| (b.0, b.3.len())).collect::<Vec<_>>()
    };
    let material = AssetId::new(AssetType::Material, "env/rock_wet");
    act(&mut app, Action::OpenAsset(material.clone()));
    let m = material.source_rel();
    assert_eq!(generated(&mut app), [((m.clone(), Lang::Rust), 1), ((m, Lang::Wesl), 1)], "icon-only, one icon each");
    let tips: Vec<String> = {
        let w = app.world_mut();
        let mut q = w.query::<(&shader_editor_app::ui::OnActivate, &Tooltip)>();
        q.iter(w).filter(|(a, _)| matches!(a.0, Action::ShowGenerated(..))).map(|(_, t)| t.0.clone()).collect()
    };
    assert!(tips.contains(&"Show generated WESL".to_string()) && tips.contains(&"Show generated Rust".to_string()));

    let function = AssetId::new(AssetType::Function, "common/wet_mask");
    act(&mut app, Action::OpenAsset(function.clone()));
    assert_eq!(generated(&mut app), [((function.source_rel(), Lang::Wesl), 1)], "a function has no Rust");
    // A function has no preview: no Preview mode group.
    assert!(
        topbar_buttons(&mut app, |a| matches!(a, Action::SetValue { target: PropTarget::PreviewMode, .. })
            .then_some(()))
        .is_empty()
    );
}

/// Design §5.4: the Preview mode group (Unlit / Lit / Full) shows the stored mode, and is disabled — its icons
/// dimmed, the stored mode's less — until a node is previewed.
#[test]
fn preview_group_disabled_without_preview() {
    use bevy::feathers::controls::ButtonVariant;
    use shader_editor_app::ui::areas::{AreasPlugin, editor_topbar};
    use shader_graph_core::Value;

    let (_dir, mut app) = app();
    app.add_plugins(AreasPlugin);
    app.world_mut().spawn_scene(editor_topbar()).unwrap();
    act(&mut app, Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet")));
    let group = |app: &mut App| {
        let mut found = topbar_buttons(app, |a| match a {
            Action::SetValue { target: PropTarget::PreviewMode, value: Value::Str(s), .. } => Some(s.clone()),
            _ => None,
        });
        let order = ["Unlit", "Lit", "Full"];
        found.sort_by_key(|b| order.iter().position(|o| *o == b.0));
        found
    };
    let modes = group(&mut app);
    assert_eq!(modes.iter().map(|b| b.0.as_str()).collect::<Vec<_>>(), ["Unlit", "Lit", "Full"]);
    for (mode, disabled, variant, tints) in &modes {
        assert!(*disabled, "{mode}: no node previewed");
        let selected = mode == "Unlit";
        assert_eq!(*variant, ButtonVariant::selected(selected), "{mode}: the stored mode shows");
        // Dimmed, the stored mode's icon only a little so it stays clearly visible.
        let dimmed = |c: &Color| if selected { (0.6..1.0).contains(&c.alpha()) } else { c.alpha() < 0.5 };
        assert!(!tints.is_empty() && tints.iter().all(dimmed), "{mode}: dimmed icon {tints:?}");
    }

    act(&mut app, Action::AddNode { kind: "stdlib::Lerp".into(), pos: (-700.0, 0.0), link: None, params: Vec::new() });
    let lerp = {
        let d = app.world().resource::<Editor>().doc().unwrap();
        d.asset.nodes.iter().find(|(_, n)| n.kind == "stdlib::Lerp").unwrap().0.clone()
    };
    act(&mut app, Action::TogglePreview(lerp.clone()));
    for (mode, disabled, _, tints) in group(&mut app) {
        assert!(!disabled, "{mode}: a node is previewed");
        assert!(tints.iter().all(|c| c.alpha() == 1.0), "{mode}: full icon {tints:?}");
    }
    act(&mut app, Action::TogglePreview(lerp));
    assert!(group(&mut app).iter().all(|b| b.1), "preview off: disabled again");
}

/// [`app`] that types: keys reach the focused field (and the hotkey contexts), number inputs work as in the GUI
/// (pointer focus, text editing), and their changes become actions.
fn typing_app() -> (tempfile::TempDir, App) {
    typing_app_with(|_| {})
}

/// [`typing_app`] with `extra` plugins added before the app is finished.
fn typing_app_with(extra: impl FnOnce(&mut App)) -> (tempfile::TempDir, App) {
    use bevy::input_focus::InputDispatchPlugin;
    use bevy::input_focus::pointer_focus::PointerFocusPlugin;
    use bevy_enhanced_input::prelude::EnhancedInputPlugin;
    use shader_editor_app::editors::main_input::InputContextsPlugin;
    use shader_editor_app::ui::props::PropsPlugin;

    let (dir, mut app) = app_with(|app| {
        app.add_plugins((
            EnhancedInputPlugin,
            InputDispatchPlugin,
            PointerFocusPlugin,
            // As in the GUI: focus changes become `FocusGained` / `FocusLost`; Tab / Shift+Tab move the focus.
            bevy::input_focus::InputFocusPlugin,
            bevy::input_focus::tab_navigation::TabNavigationPlugin,
            bevy::text::TextPlugin,
            bevy::ui_widgets::TextInputPlugin,
        ));
        // Read by the text input; written by winit and bevy_picking in the GUI.
        app.add_message::<bevy::window::Ime>().add_message::<bevy::picking::events::PointerRelease>();
        app.init_resource::<UiScale>().init_resource::<PointerState>();
        app.add_plugins((InputContextsPlugin, PropsPlugin));
        // Focused input goes to the focused entity only while there is a primary window.
        app.world_mut().spawn((Window::default(), bevy::window::PrimaryWindow));
        extra(app);
        app.finish();
    });
    app.update();
    (dir, app)
}

/// A key as winit sends it: the physical key, its logical key and the text it types.
fn key(app: &mut App, key_code: KeyCode, logical_key: bevy::input::keyboard::Key, text: Option<&str>) {
    use bevy::input::ButtonState;
    use bevy::input::keyboard::KeyboardInput;
    for state in [ButtonState::Pressed, ButtonState::Released] {
        let text = text.filter(|_| state == ButtonState::Pressed).map(Into::into);
        let logical_key = logical_key.clone();
        app.world_mut().write_message(KeyboardInput {
            key_code,
            logical_key,
            state,
            text,
            repeat: false,
            window: Entity::PLACEHOLDER,
        });
        app.update();
        app.update();
    }
}

fn type_char(app: &mut App, c: char, key_code: KeyCode) {
    let s = c.to_string();
    key(app, key_code, bevy::input::keyboard::Key::Character(s.as_str().into()), Some(&s));
}

/// The number input's scrubber: the invisible child over its text that turns presses into drags or text editing.
fn scrubber(app: &mut App, root: Entity) -> Entity {
    let text = text_in(app, root);
    app.world().get::<Children>(text).unwrap()[0]
}

/// Puts the inline number input of `node.pin` into text editing, as a click on it does (press + release without
/// movement). Without fonts the field has no text layout here, so edits that need one (select all, Backspace) do
/// nothing; typed text goes after what the field shows.
fn edit_number(app: &mut App, node: &str, pin: &str) -> Entity {
    let root = inline_entity(app, node, pin).unwrap();
    assert!(app.world().get::<bevy::feathers::controls::FeathersNumberInput>(root).is_some(), "a number input");
    let s = scrubber(app, root);
    click(app, s);
    app.update();
    app.update();
    let text = text_in(app, root);
    assert_eq!(app.world().resource::<bevy::input_focus::InputFocus>().get(), Some(text), "the click focuses it");
    root
}

/// Review Focus 4: a value typed beyond a soft limit (pin range `0..1`) is kept, not clamped.
#[test]
fn typed_number_beyond_soft_limit_is_kept() {
    let (_dir, mut app) = typing_app();
    act(&mut app, Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet")));
    act(&mut app, Action::AddNode { kind: "stdlib::Lerp".into(), pos: (-600.0, 0.0), link: None, params: Vec::new() });
    let lerp = added(&app, "stdlib::Lerp", (-600.0, 0.0));
    let alpha = PropTarget::Input { node: lerp.clone(), pin: "alpha".into() };
    act(&mut app, Action::SetComponent { target: alpha, comp: 0, value: 0.0, is_final: true });
    let root = edit_number(&mut app, &lerp, "alpha");
    assert!(app.world().get::<bevy::feathers::controls::SoftLimit>(root).is_some(), "alpha has a soft limit");
    // "0" + "5": the field holds "05".
    type_char(&mut app, '5', KeyCode::Digit5);
    key(&mut app, KeyCode::Enter, bevy::input::keyboard::Key::Enter, None);
    let d = app.world().resource::<Editor>().doc().unwrap();
    assert_eq!(d.asset.nodes[&lerp].inputs.get("alpha"), Some(&shader_graph_core::Value::Float(5.0)));
}

/// Review Focus 1: keys typed into a number input in text editing reach the field, not the hotkeys (`C` adds a
/// comment, Delete deletes the selection); once the focus leaves, the same key acts.
#[test]
fn typing_in_number_input_fires_no_hotkey() {
    let (_dir, mut app) = typing_app();
    act(&mut app, Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet")));
    act(&mut app, Action::AddNode { kind: "stdlib::Lerp".into(), pos: (-600.0, 0.0), link: None, params: Vec::new() });
    let lerp = added(&app, "stdlib::Lerp", (-600.0, 0.0));
    act(&mut app, Action::Select([lerp.clone()].into()));
    let shape = |app: &App| {
        let d = app.world().resource::<Editor>().doc().unwrap();
        (d.asset.nodes.len(), d.asset.comments.len())
    };
    let before = shape(&app);
    let root = edit_number(&mut app, &lerp, "alpha");
    type_char(&mut app, 'c', KeyCode::KeyC);
    let text = text_in(&mut app, root);
    assert_eq!(app.world().get::<bevy::text::EditableText>(text).unwrap().value().to_string(), "0.5c", "typed");
    key(&mut app, KeyCode::Delete, bevy::input::keyboard::Key::Delete, None);
    assert_eq!(shape(&app), before, "no comment added, no node deleted");

    app.world_mut().resource_mut::<bevy::input_focus::InputFocus>().clear();
    app.update();
    app.update();
    key(&mut app, KeyCode::Delete, bevy::input::keyboard::Key::Delete, None);
    assert_eq!(shape(&app), (before.0 - 1, before.1), "the hotkeys work again");
}

/// Design §6.4: a Named Route picked in the palette with Enter opens the rename dialog while Enter may still be held;
/// its key repeat must not submit the default name, only a fresh press does.
#[test]
fn dialog_enter_ignores_key_repeat() {
    use bevy::input::ButtonState;
    use bevy::input::keyboard::{Key, KeyboardInput};

    let (_dir, mut app) = typing_app_with(|app| {
        app.add_plugins(shader_editor_app::ui::dialog::DialogPlugin);
    });
    act(&mut app, Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet")));
    let param = {
        let d = app.world().resource::<Editor>().doc().unwrap();
        d.asset.nodes.iter().find(|(_, n)| n.kind == "param::Scalar").unwrap().0.clone()
    };
    act(&mut app, Action::TextDialog(PropTarget::Param { node: param, key: "name".into() }));
    app.update();
    let dialog_open = |app: &App| app.world().resource::<Editor>().dialog.is_some();
    assert!(dialog_open(&app));
    let w = app.world_mut();
    let field = w.query_filtered::<Entity, With<shader_editor_app::ui::dialog::DialogInput>>().single(w).unwrap();
    assert_eq!(app.world().resource::<bevy::input_focus::InputFocus>().get(), Some(field), "the field has focus");
    app.world_mut().write_message(KeyboardInput {
        key_code: KeyCode::Enter,
        logical_key: Key::Enter,
        state: ButtonState::Pressed,
        text: None,
        repeat: true,
        window: Entity::PLACEHOLDER,
    });
    app.update();
    app.update();
    assert!(dialog_open(&app), "a repeated Enter does not submit");
    key(&mut app, KeyCode::Enter, Key::Enter, None);
    assert!(!dialog_open(&app), "a fresh Enter submits");
}

/// The mouse button as winit reports it (what `ButtonInput<MouseButton>` follows).
fn mouse(app: &mut App, state: bevy::input::ButtonState) {
    app.world_mut().write_message(bevy::input::mouse::MouseButtonInput {
        button: MouseButton::Left,
        state,
        window: Entity::PLACEHOLDER,
    });
    app.update();
}

/// A scrub drag on a node's number input focuses its text (pointer focus) without editing it: hotkeys stay off while
/// the button is held; once it is released the canvas catches up (a read-only focused text is not typing) and the
/// hotkeys work. The drag is confined to the soft limit and is one undo step.
#[test]
fn scrub_drag_gates_hotkeys_only_while_held() {
    use bevy::camera::NormalizedRenderTarget;
    use bevy::picking::backend::HitData;
    use bevy::picking::events::{PointerDrag, PointerDragEnd, PointerDragStart, PointerPress, PointerRelease};
    use bevy::picking::pointer::{Location, PointerButton, PointerId};

    let (_dir, mut app) = typing_app();
    act(&mut app, Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet")));
    act(&mut app, Action::AddNode { kind: "stdlib::Lerp".into(), pos: (-600.0, 0.0), link: None, params: Vec::new() });
    let lerp = added(&app, "stdlib::Lerp", (-600.0, 0.0));
    act(&mut app, Action::Select([lerp.clone()].into()));
    let nodes = |app: &App| app.world().resource::<Editor>().doc().unwrap().asset.nodes.len();
    let alpha =
        |app: &App| app.world().resource::<Editor>().doc().unwrap().asset.nodes[&lerp].inputs.get("alpha").cloned();
    let before = nodes(&app);
    // The node's position in the model and on the canvas: scrubbing a field on it must not drag the node.
    let model_pos = |app: &App| app.world().resource::<Editor>().doc().unwrap().asset.nodes[&lerp].pos;
    let shown_pos = |app: &mut App| {
        let w = app.world_mut();
        let mut q = w.query::<(&NodeKey, &NodePosition)>();
        q.iter(w).find(|(k, _)| k.0 == lerp).map(|(_, p)| p.0).unwrap()
    };
    let (pos, shown) = (model_pos(&app), shown_pos(&mut app));
    let root = inline_entity(&mut app, &lerp, "alpha").unwrap();
    let (text, s) = (text_in(&mut app, root), scrubber(&mut app, root));

    let target = NormalizedRenderTarget::None { width: 800, height: 600 };
    let pointer = Pointer::new(PointerId::Mouse, Location { target, position: Vec2::ZERO });
    let hit = HitData::new(Entity::PLACEHOLDER, 0.0, None, None);
    let button = PointerButton::Primary;
    app.world_mut().register_component::<Window>();
    mouse(&mut app, bevy::input::ButtonState::Pressed);
    let w = app.world_mut();
    w.trigger(PointerPress { entity: s, pointer: pointer.clone(), button, hit: hit.clone(), count: 1 });
    w.flush();
    w.trigger(PointerDragStart { entity: s, pointer: pointer.clone(), button, hit: hit.clone() });
    w.flush();
    let by = Vec2::new(40.0, 0.0);
    w.trigger(PointerDrag { entity: s, pointer: pointer.clone(), button, distance: by, delta: by });
    w.flush();
    app.update();
    assert_eq!(app.world().resource::<bevy::input_focus::InputFocus>().get(), Some(text), "the press focuses the text");
    key(&mut app, KeyCode::Delete, bevy::input::keyboard::Key::Delete, None);
    assert_eq!(nodes(&app), before, "no hotkey during the drag");
    assert_eq!(shown_pos(&mut app), shown, "the node does not move on the canvas during the drag");

    let w = app.world_mut();
    w.trigger(PointerDragEnd { entity: s, pointer: pointer.clone(), button, distance: by });
    w.flush();
    w.trigger(PointerRelease { entity: s, pointer, button, hit });
    w.flush();
    mouse(&mut app, bevy::input::ButtonState::Released);
    app.update();
    assert_eq!(alpha(&app), Some(shader_graph_core::Value::Float(1.0)), "dragged to the end of the soft limit");
    assert!(app.world().get_entity(root).is_err(), "rebuilt after the drag");
    assert_eq!((model_pos(&app), shown_pos(&mut app)), (pos, shown), "the node stays where it was");
    act(&mut app, Action::Undo);
    assert_eq!(alpha(&app), None, "the drag is one undo step");
    key(&mut app, KeyCode::Delete, bevy::input::keyboard::Key::Delete, None);
    assert_eq!(nodes(&app), before - 1, "hotkeys work after the drag");
}

/// Drag precision: ranged fields (former sliders) round drags to 2 decimals; unranged fields and vector components
/// keep 3 (Feathers derives an unranged drag speed from it: `10^-precision` per pixel).
#[test]
fn number_precision_by_kind() {
    use bevy::feathers::controls::{NumberInputPrecision, SoftLimit};

    let (_dir, mut app) = app();
    act(&mut app, Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet")));
    act(&mut app, Action::AddNode { kind: "stdlib::Lerp".into(), pos: (-600.0, 0.0), link: None, params: Vec::new() });
    let lerp = added(&app, "stdlib::Lerp", (-600.0, 0.0));
    let precision = |app: &mut App, pin: &str, comp: usize| {
        let want = PropTarget::Input { node: lerp.clone(), pin: pin.into() };
        let w = app.world_mut();
        let mut q = w.query::<(&PropField, &NumberInputPrecision, Has<SoftLimit>)>();
        q.iter(w).find(|(f, ..)| f.target == want && f.comp == comp).map(|(_, p, soft)| (p.0, soft)).unwrap()
    };
    assert_eq!(precision(&mut app, "alpha", 0), (2, true), "ranged");
    assert_eq!(precision(&mut app, "a", 0), (3, false), "unranged");
    act(&mut app, Action::SetPinType { node: lerp.clone(), ty: shader_graph_core::PinType::Float2 });
    assert_eq!(precision(&mut app, "a", 1), (3, false), "a vector component");
}

/// View → Show console off (design §3): the console/logs keeps only its header bar (the counts and its buttons), the body
/// is not displayed and the container shrinks to the header.
#[test]
fn hidden_log_keeps_header() {
    use shader_editor_app::ui::log::{LogBody, LogHeader, LogPane, LogPlugin, console};

    let (_dir, mut app) = app();
    app.add_plugins(LogPlugin);
    app.world_mut().spawn_scene(console()).unwrap();
    app.update();
    // (container height, display of the pane body around `LogBody`, display of the pane header around `LogHeader`)
    let state = |app: &mut App| {
        let w = app.world_mut();
        let height = w.query_filtered::<&Node, With<LogPane>>().single(w).unwrap().height;
        let body = w.query_filtered::<&ChildOf, With<LogBody>>().single(w).unwrap().parent();
        let header = w.query_filtered::<&ChildOf, With<LogHeader>>().single(w).unwrap().parent();
        let display = |e: Entity| w.get::<Node>(e).unwrap().display;
        (height, display(body), display(header))
    };
    let (height, body, header) = state(&mut app);
    assert!(matches!(height, Val::Px(h) if h > 100.0), "{height:?}");
    assert_eq!((body, header), (Display::Flex, Display::Flex));

    act(&mut app, Action::ToggleLog);
    assert!(!app.world().resource::<Editor>().config.show_log);
    assert_eq!(state(&mut app), (Val::Auto, Display::None, Display::Flex), "hidden: the header only");
    // The header still counts the problems.
    let w = app.world_mut();
    let title = w.query_filtered::<&Text, With<LogHeader>>().single(w).unwrap().0.clone();
    assert!(title.starts_with("Log - ") && title.contains("error"), "{title}");

    act(&mut app, Action::ToggleLog);
    assert_eq!(state(&mut app), (height, Display::Flex, Display::Flex), "shown again");
}

/// View → Show properties off (design §4): the right sidebar is not displayed, whatever the active kind.
#[test]
fn hidden_properties_hides_right_sidebar() {
    use shader_editor_app::ui::areas::{AreasPlugin, RightSidebar, right_sidebar};

    let (_dir, mut app) = app();
    app.add_plugins(AreasPlugin);
    app.world_mut().spawn_scene(right_sidebar()).unwrap();
    act(&mut app, Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet")));
    let sidebar = |app: &mut App| {
        let w = app.world_mut();
        w.query_filtered::<&Node, With<RightSidebar>>().single(w).unwrap().display
    };
    assert_eq!(sidebar(&mut app), Display::Flex);
    act(&mut app, Action::ToggleProperties);
    assert_eq!(sidebar(&mut app), Display::None, "graph tab, properties hidden");
    act(&mut app, Action::CloseTab(usize::MAX));
    assert_eq!(sidebar(&mut app), Display::None, "no tab, properties hidden");
    act(&mut app, Action::ToggleProperties);
    assert_eq!(sidebar(&mut app), Display::Flex, "no tab: the placeholder");
    act(&mut app, Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet")));
    assert_eq!(sidebar(&mut app), Display::Flex);
}

/// What the [`focus_app`] buttons and hotkeys did: the actions of the activated buttons, in order, and how often the
/// graph's palette hotkey (Space) started.
#[derive(Resource, Default)]
struct Hits {
    buttons: Vec<Action>,
    palette: usize,
}

/// [`typing_app`] with the editor topbar under a `TabGroup` (as under the GUI's root), Bevy's button behaviour
/// (pointer and keyboard activation) and [`Hits`]; rock_wet is open.
fn focus_app() -> (tempfile::TempDir, App, Entity) {
    focus_app_with(|_| {})
}

/// [`focus_app`] with `extra` plugins.
fn focus_app_with(extra: impl FnOnce(&mut App)) -> (tempfile::TempDir, App, Entity) {
    use bevy::input_focus::tab_navigation::TabGroup;
    use bevy::ui_widgets::Activate;
    use bevy_enhanced_input::prelude::Start;
    use shader_editor_app::editors::graph::OpenPalette;
    use shader_editor_app::ui::OnActivate;
    use shader_editor_app::ui::areas::{AreasPlugin, editor_topbar};

    let (dir, mut app) = typing_app_with(|app| {
        app.add_plugins((AreasPlugin, bevy::ui_widgets::ButtonPlugin)).init_resource::<Hits>();
        app.add_observer(|ev: On<Activate>, q: Query<&OnActivate>, mut hits: ResMut<Hits>| {
            if let Ok(a) = q.get(ev.event_target()) {
                hits.buttons.push(a.0.clone());
            }
        });
        app.add_observer(|_: On<Start<OpenPalette>>, mut hits: ResMut<Hits>| hits.palette += 1);
        extra(app);
    });
    let root = app.world_mut().spawn_scene(bsn! { Node TabGroup Children [ @editor_topbar() ] }).unwrap().id();
    act(&mut app, Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet")));
    propagate_visibility(&mut app);
    app.world_mut().resource_mut::<bevy::input_focus::InputFocus>().clear();
    app.update();
    app.world_mut().resource_mut::<Hits>().buttons.clear();
    (dir, app, root)
}

/// What Bevy's visibility propagation does in the GUI (it does not run headless): `InheritedVisibility` from the
/// `Visibility` of each entity and its ancestors. Tab navigation skips hidden entities.
fn propagate_visibility(app: &mut App) {
    fn walk(w: &mut World, e: Entity, parent: bool) {
        let shown = match w.get::<Visibility>(e) {
            Some(Visibility::Visible) => true,
            Some(Visibility::Hidden) => false,
            _ => parent,
        };
        // Changed only when it changes, as Bevy's propagation (`set_if_neq`): `drop_hidden_focus` reads the change.
        if let Some(mut v) = w.get_mut::<InheritedVisibility>(e) {
            v.set_if_neq(if shown { InheritedVisibility::VISIBLE } else { InheritedVisibility::HIDDEN });
        }
        let children: Vec<Entity> = w.get::<Children>(e).map(|c| c.to_vec()).unwrap_or_default();
        for c in children {
            walk(w, c, shown);
        }
    }
    let w = app.world_mut();
    let roots: Vec<Entity> =
        w.query_filtered::<Entity, (With<InheritedVisibility>, Without<ChildOf>)>().iter(w).collect();
    for r in roots {
        walk(w, r, true);
    }
}

/// The tabbable entities under `root` in Tab order: tree order, hidden subtrees skipped (every widget here has
/// `TabIndex(0)`, asserted: another index would change the order).
fn tab_order(app: &App, root: Entity) -> Vec<Entity> {
    use bevy::input_focus::tab_navigation::TabIndex;
    fn walk(w: &World, e: Entity, out: &mut Vec<Entity>) {
        if w.get::<InheritedVisibility>(e).is_some_and(|v| !v.get()) {
            return;
        }
        if let Some(t) = w.get::<TabIndex>(e).filter(|t| t.0 >= 0) {
            assert_eq!(t.0, 0, "tab_order assumes TabIndex(0) everywhere");
            out.push(e);
        }
        for &c in w.get::<Children>(e).into_iter().flatten() {
            walk(w, c, out);
        }
    }
    let mut out = Vec::new();
    walk(app.world(), root, &mut out);
    out
}

fn focused(app: &App) -> Option<Entity> {
    app.world().resource::<bevy::input_focus::InputFocus>().get()
}

/// The topbar button that activates `action`.
fn button_of(app: &mut App, action: &Action) -> Entity {
    let w = app.world_mut();
    let mut q = w.query::<(Entity, &shader_editor_app::ui::OnActivate)>();
    q.iter(w).find(|(_, a)| a.0 == *action).map(|(e, _)| e).unwrap()
}

/// A modifier key held (`true`) or released, as winit reports it.
fn modifier(app: &mut App, key_code: KeyCode, logical_key: bevy::input::keyboard::Key, down: bool) {
    use bevy::input::ButtonState;
    use bevy::input::keyboard::KeyboardInput;
    let state = if down { ButtonState::Pressed } else { ButtonState::Released };
    let input = KeyboardInput { key_code, logical_key, state, text: None, repeat: false, window: Entity::PLACEHOLDER };
    app.world_mut().write_message(input);
    app.update();
}

fn shift(app: &mut App, down: bool) {
    modifier(app, KeyCode::ShiftLeft, bevy::input::keyboard::Key::Shift, down);
}

/// Bevy's default keyboard focus (design §2): from no focus, Tab walks every tabbable widget in order — the Feathers
/// buttons included — with the focus ring on, and Shift+Tab walks back. Neither toggles the properties.
#[test]
fn tab_reaches_buttons() {
    use bevy::input::keyboard::Key;
    use bevy::input_focus::InputFocusVisible;

    let (_dir, mut app, root) = focus_app();
    let order = tab_order(&app, root);
    assert!(order.len() >= 3, "the topbar has tabbable widgets: {order:?}");
    let mut visited = Vec::new();
    for _ in 0..order.len() {
        key(&mut app, KeyCode::Tab, Key::Tab, None);
        visited.push(focused(&app).unwrap());
    }
    assert_eq!(visited, order, "Tab follows the tab order");
    assert!(visited.iter().any(|&e| app.world().get::<bevy::ui_widgets::Button>(e).is_some()), "buttons included");
    assert!(app.world().resource::<InputFocusVisible>().0, "with the focus ring");
    shift(&mut app, true);
    key(&mut app, KeyCode::Tab, Key::Tab, None);
    shift(&mut app, false);
    assert_eq!(focused(&app), Some(order[order.len() - 2]), "Shift+Tab goes back");
    assert!(app.world().resource::<Editor>().config.show_properties, "Tab is no hotkey");
    assert!(app.world().resource::<Hits>().buttons.is_empty(), "focusing a button does not press it");
}

/// A Tab-focused button is pressed by Enter and by Space, once each; Space presses the button only, it does not open
/// the palette too (the widget wins, design §2).
#[test]
fn enter_and_space_press_focused_button() {
    use bevy::input::keyboard::Key;
    use shader_editor_app::state::ViewRequest;

    let (_dir, mut app, root) = focus_app();
    let fit_all = Action::View(ViewRequest::FitAll);
    let fit = button_of(&mut app, &fit_all);
    for _ in 0..tab_order(&app, root).len() {
        if focused(&app) == Some(fit) {
            break;
        }
        key(&mut app, KeyCode::Tab, Key::Tab, None);
    }
    assert_eq!(focused(&app), Some(fit), "Tab reaches the Fit button");
    key(&mut app, KeyCode::Enter, Key::Enter, None);
    assert_eq!(app.world().resource::<Hits>().buttons, std::slice::from_ref(&fit_all), "Enter presses it once");
    key(&mut app, KeyCode::Space, Key::Character(" ".into()), Some(" "));
    let hits = app.world().resource::<Hits>();
    assert_eq!(hits.buttons, [fit_all.clone(), fit_all], "Space presses it once");
    assert_eq!(hits.palette, 0, "Space does not open the palette");
    assert_eq!(focused(&app), Some(fit), "the button keeps the focus");
}

/// A clicked toolbar button keeps the focus (Bevy's pointer focus): Space presses it again, not the palette. A click
/// on the canvas clears the focus (Bevy's "click outside to unfocus"), and Space opens the palette again.
#[test]
fn space_opens_palette_after_canvas_click() {
    use bevy::input::keyboard::Key;
    use shader_editor_app::state::ViewRequest;

    let (_dir, mut app, _) = focus_app();
    let fit_all = Action::View(ViewRequest::FitAll);
    let fit = button_of(&mut app, &fit_all);
    click(&mut app, fit);
    app.update();
    assert_eq!(app.world().resource::<Hits>().buttons, std::slice::from_ref(&fit_all), "the click presses it");
    assert_eq!(focused(&app), Some(fit), "and focuses it");
    key(&mut app, KeyCode::Space, Key::Character(" ".into()), Some(" "));
    assert_eq!(app.world().resource::<Hits>().buttons, [fit_all.clone(), fit_all.clone()], "Space presses it");
    assert_eq!(app.world().resource::<Hits>().palette, 0, "not the palette");

    let canvas = app.world().resource::<EditorCanvas>().canvas;
    click(&mut app, canvas);
    app.update();
    assert_eq!(focused(&app), None, "a click on the canvas clears the focus");
    key(&mut app, KeyCode::Space, Key::Character(" ".into()), Some(" "));
    let hits = app.world().resource::<Hits>();
    assert_eq!(hits.palette, 1, "Space opens the palette");
    assert_eq!(hits.buttons, [fit_all.clone(), fit_all], "the button does not act again");
}

/// The entity is `root` or inside it.
fn inside(app: &App, e: Entity, root: Entity) -> bool {
    let w = app.world();
    std::iter::successors(Some(e), |x| w.get::<ChildOf>(*x).map(ChildOf::parent)).any(|x| x == root)
}

/// Tab inside an overlay stays inside it (a modal `TabGroup`): a dialog (Feathers' `ModalDialog`) cycles over its
/// field and buttons, and Space / Enter press nothing behind it.
#[test]
fn tab_stays_inside_dialog() {
    use bevy::input::keyboard::Key;
    use shader_editor_app::ui::dialog::DialogRoot;

    let (_dir, mut app, root) = focus_app_with(|app| {
        app.add_plugins((
            shader_editor_app::ui::dialog::DialogPlugin,
            bevy::ui_widgets::DialogPlugin,
            bevy::ui_widgets::ModalDialogPlugin,
        ));
    });
    let background = tab_order(&app, root);
    let param = {
        let d = app.world().resource::<Editor>().doc().unwrap();
        d.asset.nodes.iter().find(|(_, n)| n.kind == "param::Scalar").unwrap().0.clone()
    };
    act(&mut app, Action::TextDialog(PropTarget::Param { node: param, key: "name".into() }));
    propagate_visibility(&mut app);
    let dialog = {
        let w = app.world_mut();
        w.query_filtered::<Entity, With<DialogRoot>>().single(w).unwrap()
    };
    let mut seen = Vec::new();
    for i in 0..5 {
        key(&mut app, KeyCode::Tab, Key::Tab, None);
        let f = focused(&app).unwrap();
        assert!(inside(&app, f, dialog), "Tab {i} stays in the dialog: {f:?}");
        seen.push(f);
    }
    seen.sort();
    seen.dedup();
    assert!(seen.len() >= 2, "Tab reaches the dialog's buttons too: {seen:?}");
    assert!(seen.iter().all(|f| !background.contains(f)));
    key(&mut app, KeyCode::Space, Key::Character(" ".into()), Some(" "));
    let hits = app.world().resource::<Hits>();
    assert!(hits.buttons.iter().all(|a| matches!(a, Action::CloseDialog)), "nothing behind it: {:?}", hits.buttons);
    assert_eq!(hits.palette, 0);
}

/// Tab in the palette stays in its field (a modal `TabGroup` on `PaletteRoot`): Enter picks the highlighted node and
/// presses no button behind the palette.
#[test]
fn tab_stays_inside_palette() {
    use bevy::input::keyboard::Key;
    use shader_editor_app::ui::palette::{Palette, PalettePlugin, PaletteRoot};

    let (_dir, mut app, _) = focus_app_with(|app| {
        app.add_plugins(PalettePlugin);
    });
    app.world_mut().resource_mut::<Palette>().show(Vec2::new(100.0, 100.0), Vec2::ZERO, None);
    app.update();
    app.update();
    propagate_visibility(&mut app);
    let palette = {
        let w = app.world_mut();
        w.query_filtered::<Entity, With<PaletteRoot>>().single(w).unwrap()
    };
    assert!(focused(&app).is_some_and(|f| inside(&app, f, palette)), "the palette's field has the focus");
    for i in 0..3 {
        key(&mut app, KeyCode::Tab, Key::Tab, None);
        let f = focused(&app);
        assert!(f.is_some_and(|f| inside(&app, f, palette)), "Tab {i} stays in the palette: {f:?}");
    }
    let nodes = |app: &App| app.world().resource::<Editor>().doc().unwrap().asset.nodes.len();
    let before = nodes(&app);
    key(&mut app, KeyCode::Enter, Key::Enter, None);
    assert_eq!(nodes(&app), before + 1, "Enter picks a node");
    assert_eq!(app.world().resource::<Hits>().buttons, [], "and presses no button behind the palette");
}

/// Ctrl+Tab is Next tab, never Bevy's tab navigation too (the one exception to its default, design §2): after it the
/// focus is cleared, so Space opens the palette and presses no widget.
#[test]
fn ctrl_tab_moves_no_focus() {
    use bevy::input::keyboard::Key;

    let (_dir, mut app, _) = focus_app();
    act(&mut app, Action::OpenAsset(AssetId::new(AssetType::Material, "env/water")));
    propagate_visibility(&mut app);
    modifier(&mut app, KeyCode::ControlLeft, Key::Control, true);
    for _ in 0..3 {
        key(&mut app, KeyCode::Tab, Key::Tab, None);
    }
    modifier(&mut app, KeyCode::ControlLeft, Key::Control, false);
    assert!(app.world().resource::<Editor>().doc().is_some(), "a graph tab is active");
    assert_eq!(focused(&app), None, "no focus left");
    assert!(!app.world().resource::<bevy::input_focus::InputFocusVisible>().0, "no focus ring");
    key(&mut app, KeyCode::Space, Key::Character(" ".into()), Some(" "));
    let hits = app.world().resource::<Hits>();
    assert_eq!(hits.palette, 1, "Space opens the palette");
    assert_eq!(hits.buttons, [], "no widget pressed");
}

/// Inactive editor areas are hidden (`Visibility::Hidden` next to `Display::None`), so Tab never reaches a widget in
/// them: Bevy's tab navigation skips hidden entities, not `Display::None` ones.
#[test]
fn tab_skips_inactive_areas() {
    use bevy::input::keyboard::Key;
    use bevy::input_focus::tab_navigation::TabIndex;
    use shader_editor_app::editors::AreaOf;
    use shader_editor_app::tab::ViewKind;

    let (_dir, mut app, root) = focus_app();
    let curve = {
        let w = app.world_mut();
        w.query::<(Entity, &AreaOf)>().iter(w).find(|(_, a)| a.0 == ViewKind::Curve).unwrap().0
    };
    let stray = app.world_mut().spawn((Node::default(), bevy::ui_widgets::Button, TabIndex(0), ChildOf(curve))).id();
    propagate_visibility(&mut app);
    let order = tab_order(&app, root);
    for _ in 0..order.len() + 1 {
        key(&mut app, KeyCode::Tab, Key::Tab, None);
        assert_ne!(focused(&app), Some(stray), "Tab skips the curve editor's hidden topbar");
    }
}

/// A focused widget hidden by a panel toggle loses the focus (`drop_hidden_focus`): a right sidebar checkbox has it,
/// Ctrl+3 hides the sidebar, and Space then opens the palette instead of toggling the invisible checkbox.
#[test]
fn hiding_a_panel_drops_its_focus() {
    use bevy::input::keyboard::Key;
    use bevy::ui_widgets::Checkbox;
    use shader_editor_app::ui::areas::{RightSidebar, right_sidebar};

    let (_dir, mut app, _) = focus_app();
    app.world_mut().spawn_scene(right_sidebar()).unwrap();
    act(&mut app, Action::OpenAsset(AssetId::new(AssetType::Material, "env/water")));
    propagate_visibility(&mut app);
    app.update();
    let revision = |app: &App| app.world().resource::<Editor>().doc().unwrap().revision;
    let before = revision(&app);
    let checkbox = {
        let w = app.world_mut();
        let sidebar = w.query_filtered::<Entity, With<RightSidebar>>().single(w).unwrap();
        let boxes: Vec<Entity> = w.query_filtered::<Entity, With<Checkbox>>().iter(w).collect();
        let parents = |e: Entity| std::iter::successors(Some(e), |x| w.get::<ChildOf>(*x).map(ChildOf::parent));
        boxes.into_iter().find(|b| parents(*b).any(|x| x == sidebar)).expect("a checkbox in the right sidebar")
    };
    app.world_mut()
        .resource_mut::<bevy::input_focus::InputFocus>()
        .set(checkbox, bevy::input_focus::FocusCause::Navigated);
    app.update();
    propagate_visibility(&mut app);
    app.update();
    assert_eq!(focused(&app), Some(checkbox), "a visible focused widget keeps the focus");

    modifier(&mut app, KeyCode::ControlLeft, Key::Control, true);
    key(&mut app, KeyCode::Digit3, Key::Character("3".into()), None);
    modifier(&mut app, KeyCode::ControlLeft, Key::Control, false);
    assert!(!app.world().resource::<Editor>().config.show_properties, "Ctrl+3 hid the right sidebar");
    propagate_visibility(&mut app);
    app.update();
    assert_eq!(focused(&app), None, "the hidden checkbox lost the focus");

    key(&mut app, KeyCode::Space, Key::Character(" ".into()), Some(" "));
    let hits = app.world().resource::<Hits>();
    assert_eq!(hits.palette, 1, "Space opens the palette");
    assert_eq!(hits.buttons, [], "no widget pressed");
    assert_eq!(revision(&app), before, "no edit: the invisible checkbox stayed as it was");
}

/// Tab onto a number input that was used before (its text `Static`, as Feathers leaves it after a scrub, Enter or
/// focus loss) makes it typable, like a fresh one: the keys edit it and reach no hotkey (decision 0034). Tab walks
/// from a field over a button (which keeps the focus, design §2) to it.
#[test]
fn tab_onto_used_number_input_types_into_it() {
    use bevy::feathers::controls::FeathersNumberInput;
    use bevy::input::keyboard::Key;
    use bevy::input_focus::InputFocus;
    use bevy::input_focus::tab_navigation::{TabGroup, TabIndex};
    use bevy::text::{EditableText, TextReadWriteMode};
    use shader_editor_app::editors::main_input::KeyboardGate;

    let (_dir, mut app) = typing_app();
    act(&mut app, Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet")));
    act(&mut app, Action::AddNode { kind: "stdlib::Lerp".into(), pos: (-600.0, 0.0), link: None, params: Vec::new() });
    let lerp = added(&app, "stdlib::Lerp", (-600.0, 0.0));
    act(&mut app, Action::Select([lerp].into()));
    let nodes = |app: &App| app.world().resource::<Editor>().doc().unwrap().asset.nodes.len();
    let before = nodes(&app);

    let shown = InheritedVisibility::VISIBLE;
    let group = app.world_mut().spawn((Node::default(), TabGroup::default())).id();
    let field = (EditableText::default(), TabIndex(0));
    let field = app.world_mut().spawn((Node::default(), shown, field, ChildOf(group))).id();
    let button = (bevy::ui_widgets::Button, TabIndex(0));
    let button = app.world_mut().spawn((Node::default(), shown, button, ChildOf(group))).id();
    let number = app.world_mut().spawn_scene(bsn! { @FeathersNumberInput {} ChildOf(group) }).unwrap().id();
    app.update();
    let text = text_in(&mut app, number);
    app.world_mut().entity_mut(number).insert(shown);
    app.world_mut().entity_mut(text).insert((shown, TextReadWriteMode::Static, TabIndex(1)));
    app.world_mut().resource_mut::<InputFocus>().set(field, bevy::input_focus::FocusCause::Pressed);
    app.update();
    assert!(app.world().resource::<KeyboardGate>().typing, "typing into the first field");

    key(&mut app, KeyCode::Tab, Key::Tab, None);
    assert_eq!(app.world().resource::<InputFocus>().get(), Some(button), "Tab moves to the button");
    assert!(!app.world().resource::<KeyboardGate>().typing, "a focused button is no typing");
    key(&mut app, KeyCode::Tab, Key::Tab, None);
    assert_eq!(app.world().resource::<InputFocus>().get(), Some(text), "Tab moves on to the number input");
    assert!(app.world().resource::<Editor>().config.show_properties, "Tab is no hotkey");
    assert!(app.world().resource::<KeyboardGate>().typing, "the Tab-focused number input takes the keyboard");
    type_char(&mut app, '5', KeyCode::Digit5);
    key(&mut app, KeyCode::Delete, Key::Delete, None);
    type_char(&mut app, 'c', KeyCode::KeyC);
    assert_eq!(nodes(&app), before, "Delete / C reach no graph hotkey");
    // Without fonts there is no text layout here, so the select-all does nothing: typed text goes after the "0".
    assert_eq!(app.world().get::<EditableText>(text).unwrap().value().to_string(), "05c", "the keys edit it");
}

/// A real click into a field (picking's `PointerPress` inside the frame, after the typing gate ran) focuses it, also
/// with nothing focused before, and the field takes the keyboard.
#[test]
fn click_into_field_keeps_focus() {
    use bevy::camera::NormalizedRenderTarget;
    use bevy::input_focus::InputFocus;
    use bevy::input_focus::tab_navigation::{TabGroup, TabIndex};
    use bevy::picking::backend::HitData;
    use bevy::picking::events::PointerPress;
    use bevy::picking::pointer::{Location, PointerButton, PointerId};
    use shader_editor_app::editors::main_input::{InputSync, KeyboardGate};

    #[derive(Resource, Default)]
    struct PressNext(Option<Entity>);

    let (_dir, mut app) = typing_app();
    app.world_mut().register_component::<Window>();
    app.init_resource::<PressNext>().add_systems(
        PreUpdate,
        (|mut next: ResMut<PressNext>, mut commands: Commands| {
            let Some(entity) = next.0.take() else { return };
            let target = NormalizedRenderTarget::None { width: 800, height: 600 };
            let pointer = Pointer::new(PointerId::Mouse, Location { target, position: Vec2::ZERO });
            let hit = HitData::new(Entity::PLACEHOLDER, 0.0, None, None);
            commands.trigger(PointerPress { entity, pointer, button: PointerButton::Primary, hit, count: 1 });
        })
        .after(InputSync),
    );
    let group = app.world_mut().spawn((Node::default(), TabGroup::default())).id();
    let field = (bevy::text::EditableText::default(), TabIndex(0));
    let field = app.world_mut().spawn((Node::default(), InheritedVisibility::VISIBLE, field, ChildOf(group))).id();
    app.world_mut().resource_mut::<InputFocus>().clear();
    app.update();
    assert!(!app.world().resource::<KeyboardGate>().typing);
    app.world_mut().resource_mut::<PressNext>().0 = Some(field);
    app.update();
    app.update();
    assert_eq!(app.world().resource::<InputFocus>().get(), Some(field), "the clicked field keeps the focus");
    assert!(app.world().resource::<KeyboardGate>().typing);
}

// ---------------------------------------------------------------------------------------------
// Ctrl+drag moves wires (decision 0032, design §5.3): bevy_noodle's move-wires gesture lifts the pin's wires, the
// model changes only on a drop onto a pin where the move succeeds.

/// [`gestures_app`] with bevy_noodle's pointer interaction and a laid-out canvas: 800 by 600 px, centred on the window
/// origin (no layout runs headless).
fn wires_app() -> (tempfile::TempDir, App) {
    wires_app_with(|_| {})
}

/// [`wires_app`] with `extra` plugins added after the gestures'.
fn wires_app_with(extra: impl FnOnce(&mut App)) -> (tempfile::TempDir, App) {
    let (dir, mut app) = app_with(|app| {
        app.add_message::<bevy::window::WindowCloseRequested>()
            .init_resource::<UiScale>()
            .init_resource::<shader_editor_app::ui::palette::Palette>()
            .add_plugins((NoodleInteractionPlugin, shader_editor_app::ui::input::InputPlugin));
        extra(app);
    });
    let canvas = app.world().resource::<EditorCanvas>().canvas;
    let computed = ComputedNode { size: Vec2::new(800.0, 600.0), ..default() };
    app.world_mut().entity_mut(canvas).insert(computed);
    // Pointer events bubble through `PointerTraversal`, which reads `Window`.
    app.world_mut().register_component::<Window>();
    act(&mut app, Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet")));
    (dir, app)
}

/// Adds a node of `kind` at `pos` and returns its id.
fn add_node(app: &mut App, kind: &str, pos: (f32, f32)) -> String {
    act(app, Action::AddNode { kind: kind.into(), pos, link: None, params: Vec::new() });
    added(app, kind, pos)
}

fn wire(app: &mut App, from: (&str, &str), to: (&str, &str)) {
    act(app, Action::Connect { from: (from.0.into(), from.1.into()), to: (to.0.into(), to.1.into()) });
}

/// Every wire of the model as `from.pin -> to.pin`, sorted, and the revision.
fn model_wires(app: &App) -> (Vec<String>, u64) {
    let d = app.world().resource::<Editor>().doc().unwrap();
    let mut wires: Vec<String> =
        d.asset.edges.iter().map(|e| format!("{}.{} -> {}.{}", e.from.0, e.from.1, e.to.0, e.to.1)).collect();
    wires.sort();
    (wires, d.revision)
}

/// Gives every port of the canvas a measured anchor (layout does not run headless): a node's inputs at its left
/// edge, outputs 150 to the right, one row of 20 per pin. Run again after each rebuild.
fn lay_out_pins(app: &mut App) {
    let w = app.world_mut();
    let pins: Vec<(Entity, PinKey)> = w.query::<(Entity, &PinKey)>().iter(w).map(|(e, k)| (e, k.clone())).collect();
    let nodes: Vec<(String, Vec2)> =
        w.query::<(&NodeKey, &NodePosition)>().iter(w).map(|(k, p)| (k.0.clone(), p.0)).collect();
    let mut rows: std::collections::HashMap<(String, bool), f32> = Default::default();
    for (port, key) in pins {
        let Some(origin) = nodes.iter().find(|(id, _)| *id == key.node).map(|n| n.1) else { continue };
        let row = rows.entry((key.node.clone(), key.output)).or_insert(30.0);
        let at = origin + Vec2::new(if key.output { 150.0 } else { 0.0 }, *row);
        *row += 20.0;
        w.entity_mut(port).insert(PortAnchor::new(None, at, Some(at)));
    }
    app.update();
}

/// The window position of graph point `at` (the canvas is centred on the window origin).
fn window_at(app: &App, at: Vec2) -> Vec2 {
    let canvas = app.world().resource::<EditorCanvas>().canvas;
    app.world().get::<CanvasView>(canvas).unwrap().graph_to_canvas(at) - Vec2::new(400.0, 300.0)
}

fn port_at(app: &App, port: Entity) -> Vec2 {
    app.world().get::<PortAnchor>(port).unwrap().position().unwrap()
}

/// A pointer event of a wire drag.
enum WireStep {
    Start,
    Drag,
    End,
    Cancel,
}

/// Where the pointer is: over a pin's centre, or at a graph point.
enum At {
    Pin(Entity),
    Point(Vec2),
}

/// A wire drag event of the primary button on `port`, the pointer `at`; then one update.
fn wire_step(app: &mut App, port: Entity, at: At, step: WireStep) {
    use bevy::camera::NormalizedRenderTarget;
    use bevy::picking::backend::HitData;
    use bevy::picking::events::{PointerCancel, PointerDrag, PointerDragEnd, PointerDragStart};
    use bevy::picking::pointer::{Location, PointerButton, PointerId};
    let target = NormalizedRenderTarget::None { width: 800, height: 600 };
    let at = match at {
        At::Pin(pin) => port_at(app, pin),
        At::Point(at) => at,
    };
    let position = window_at(app, at);
    let pointer = Pointer::new(PointerId::Mouse, Location { target, position });
    let hit = HitData::new(Entity::PLACEHOLDER, 0.0, None, None);
    let (button, entity, by) = (PointerButton::Primary, port, Vec2::X);
    let w = app.world_mut();
    match step {
        WireStep::Start => w.trigger(PointerDragStart { entity, pointer, button, hit }),
        WireStep::Drag => w.trigger(PointerDrag { entity, pointer, button, distance: by, delta: by }),
        WireStep::End => w.trigger(PointerDragEnd { entity, pointer, button, distance: by }),
        WireStep::Cancel => w.trigger(PointerCancel { entity, pointer, hit }),
    }
    w.flush();
    app.update();
}

/// What bevy_picking hovers: `port`, or nothing.
fn hover_only(app: &mut App, port: Option<Entity>) {
    use bevy::picking::backend::HitData;
    use bevy::picking::hover::HoverMap;
    use bevy::picking::pointer::PointerId;
    let hovered = port.map(|p| (p, HitData::new(Entity::PLACEHOLDER, 0.0, None, None))).into_iter().collect();
    app.world_mut().resource_mut::<HoverMap>().insert(PointerId::Mouse, hovered);
}

fn hold_ctrl(app: &mut App, held: bool) {
    let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
    if held { keys.press(KeyCode::ControlLeft) } else { keys.release(KeyCode::ControlLeft) }
}

/// The canvas edge between two ports, if any.
fn edge_between(app: &mut App, output: Entity, input: Entity) -> Option<Entity> {
    let w = app.world_mut();
    let mut q = w.query::<(Entity, &EdgeSource, &EdgeTarget)>();
    q.iter(w).find(|(_, s, t)| s.0 == output && t.0 == input).map(|(e, ..)| e)
}

fn geometry_of(app: &App, edge: Entity) -> EdgeGeometry {
    *app.world().get::<EdgeGeometry>(edge).unwrap()
}

fn lifted(app: &mut App) -> Vec<MovingEdges> {
    let w = app.world_mut();
    w.query::<&MovingEdges>().iter(w).cloned().collect()
}

fn pending_wires(app: &mut App) -> usize {
    let w = app.world_mut();
    w.query::<&PendingWire>().iter(w).count()
}

/// Constant → Lerp `a`, and a second Lerp to move the wire to, laid out.
fn lerps(app: &mut App) -> (String, String, String) {
    let c = add_node(app, "stdlib::Constant", (-600.0, 0.0));
    let (a, b) = (add_node(app, "stdlib::Lerp", (-300.0, 0.0)), add_node(app, "stdlib::Lerp", (-300.0, 300.0)));
    wire(app, (&c, "out"), (&a, "a"));
    lay_out_pins(app);
    (c, a, b)
}

#[test]
fn ctrl_drag_lifts_wires_without_new_wire() {
    let (_dir, mut app) = wires_app();
    let (c, a, _) = lerps(&mut app);
    let before = model_wires(&app);
    let (out, inp) = (port(&mut app, &c, "out", true), port(&mut app, &a, "a", false));
    let edge = edge_between(&mut app, out, inp).expect("the wire is on the canvas");
    hold_ctrl(&mut app, true);
    wire_step(&mut app, inp, At::Pin(inp), WireStep::Start);
    let pointer = Vec2::new(-200.0, 150.0);
    wire_step(&mut app, inp, At::Point(pointer), WireStep::Drag);
    app.update();
    assert_eq!(pending_wires(&mut app), 0, "no new wire from the pin");
    assert_eq!(lifted(&mut app).len(), 1);
    let g = geometry_of(&app, edge);
    assert!((g.end - pointer).length() < 1e-3, "the loose end is at the pointer: {}", g.end);
    assert_eq!(g.start, port_at(&app, out), "the far end stays");
    assert_eq!(model_wires(&app), before, "the model is not touched");

    // A model change while the wires are lifted waits for the gesture (no rebuild under it).
    let rev = before.1;
    act(
        &mut app,
        Action::AddNode { kind: "stdlib::Constant".into(), pos: (0.0, 600.0), link: None, params: Vec::new() },
    );
    assert!(model_wires(&app).1 > rev);
    assert!(app.world().get_entity(edge).is_ok() && lifted(&mut app).len() == 1, "not rebuilt during the gesture");
    hold_ctrl(&mut app, false);
    wire_step(&mut app, inp, At::Point(pointer), WireStep::End);
    app.update();
    assert!(app.world().get_entity(edge).is_err(), "rebuilt once the gesture ended");
}

#[test]
fn ctrl_drag_to_valid_pin_is_one_move() {
    let (_dir, mut app) = wires_app();
    let (c, a, b) = lerps(&mut app);
    let before = model_wires(&app);
    let (from, to) = (port(&mut app, &a, "a", false), port(&mut app, &b, "b", false));
    hold_ctrl(&mut app, true);
    wire_step(&mut app, from, At::Pin(from), WireStep::Start);
    hover_only(&mut app, Some(to));
    wire_step(&mut app, from, At::Pin(to), WireStep::Drag);
    assert_eq!(app.world_mut().query::<&WireTarget>().single(app.world()).unwrap().0, Some(to), "snaps to the pin");
    wire_step(&mut app, from, At::Pin(to), WireStep::End);
    hold_ctrl(&mut app, false);
    hover_only(&mut app, None);
    app.update();
    let (wires, rev) = model_wires(&app);
    assert_eq!(wires.iter().filter(|w| w.ends_with(&format!("{a}.a"))).count(), 0);
    assert!(wires.contains(&format!("{c}.out -> {b}.b")), "{wires:?}");
    assert_eq!(rev, before.1 + 1, "one edit");
    let (out, to) = (port(&mut app, &c, "out", true), port(&mut app, &b, "b", false));
    assert!(edge_between(&mut app, out, to).is_some(), "the canvas shows the moved wire");
    act(&mut app, Action::Undo);
    assert_eq!(model_wires(&app).0, before.0, "one undo step");
}

#[test]
fn ctrl_drag_to_empty_restores_wires() {
    let (_dir, mut app) = wires_app();
    let (c, a, _) = lerps(&mut app);
    let before = model_wires(&app);
    let (out, inp) = (port(&mut app, &c, "out", true), port(&mut app, &a, "a", false));
    let edge = edge_between(&mut app, out, inp).unwrap();
    hold_ctrl(&mut app, true);
    wire_step(&mut app, inp, At::Pin(inp), WireStep::Start);
    wire_step(&mut app, inp, At::Point(Vec2::new(-200.0, 150.0)), WireStep::Drag);
    assert_eq!(lifted(&mut app).len(), 1, "lifted mid-gesture");
    let messages = app.world().resource::<Editor>().messages.len();
    wire_step(&mut app, inp, At::Point(Vec2::new(-200.0, 150.0)), WireStep::End);
    hold_ctrl(&mut app, false);
    app.update();
    assert_eq!(model_wires(&app), before, "nothing changed");
    assert_eq!(app.world().resource::<Editor>().messages.len(), messages, "nothing to say off any pin");
    assert!(lifted(&mut app).is_empty());
    assert!(!app.world().resource::<shader_editor_app::ui::palette::Palette>().open, "no palette for a move");
    assert_eq!(geometry_of(&app, edge).end, port_at(&app, inp), "the wire is back at its pin");
}

#[test]
fn ctrl_drag_to_invalid_pin_does_not_snap_or_move() {
    let (_dir, mut app) = wires_app();
    let out_node = {
        let d = app.world().resource::<Editor>().doc().unwrap();
        d.asset.nodes.iter().find(|(_, n)| n.kind == "output::Pbr3d").unwrap().0.clone()
    };
    let v3 = add_node(&mut app, "stdlib::Constant3Vector", (-600.0, 0.0));
    let other = add_node(&mut app, "stdlib::Constant3Vector", (-600.0, 300.0));
    let lerp = add_node(&mut app, "stdlib::Lerp", (-300.0, 0.0));
    let make = add_node(&mut app, "stdlib::MakeMaterialAttributesPbr", (-600.0, 600.0));
    wire(&mut app, (&v3, "out"), (&lerp, "a"));
    wire(&mut app, (&v3, "out"), (&out_node, "base_color"));
    lay_out_pins(&mut app);
    let before = model_wires(&app);
    let from = port(&mut app, &v3, "out", true);
    let (invalid, valid) = (port(&mut app, &make, "attributes", true), port(&mut app, &other, "out", true));
    hold_ctrl(&mut app, true);
    wire_step(&mut app, from, At::Pin(from), WireStep::Start);
    let w = app.world_mut();
    let candidates = w.query::<&WireCandidates>().single(w).unwrap().clone();
    assert!(candidates.contains(&valid) && !candidates.contains(&invalid), "only pins the move succeeds on");
    hover_only(&mut app, Some(invalid));
    let at = port_at(&app, invalid);
    wire_step(&mut app, from, At::Point(at), WireStep::Drag);
    app.update();
    assert_eq!(app.world_mut().query::<&WireTarget>().single(app.world()).unwrap().0, None, "no snap");
    let lifted_edge = lifted(&mut app)[0].edges[0];
    assert!((geometry_of(&app, lifted_edge).start - at).length() < 1e-3, "follows the pointer only");
    wire_step(&mut app, from, At::Point(at), WireStep::End);
    hold_ctrl(&mut app, false);
    app.update();
    assert_eq!(model_wires(&app), before, "nothing moved");
    // Dropped on that pin: the log says why, as `MoveWires` would (decision 0032).
    let last = app.world().resource::<Editor>().messages.last().map(|m| m.text.clone()).unwrap_or_default();
    assert!(last.starts_with("Wires not moved:") && last.contains("base_color"), "{last}");
}

#[test]
fn ctrl_drag_output_moves_all_wires() {
    let (_dir, mut app) = wires_app();
    let (c, d) = (
        add_node(&mut app, "stdlib::Constant", (-600.0, 0.0)),
        add_node(&mut app, "stdlib::Constant", (-600.0, 300.0)),
    );
    let (a, b) =
        (add_node(&mut app, "stdlib::Lerp", (-300.0, 0.0)), add_node(&mut app, "stdlib::Lerp", (-300.0, 300.0)));
    wire(&mut app, (&c, "out"), (&a, "a"));
    wire(&mut app, (&c, "out"), (&b, "b"));
    lay_out_pins(&mut app);
    let before = model_wires(&app);
    let (from, to) = (port(&mut app, &c, "out", true), port(&mut app, &d, "out", true));
    hold_ctrl(&mut app, true);
    wire_step(&mut app, from, At::Pin(from), WireStep::Start);
    assert_eq!(lifted(&mut app)[0].edges.len(), 2, "every wire of the output");
    hover_only(&mut app, Some(to));
    wire_step(&mut app, from, At::Pin(to), WireStep::Drag);
    wire_step(&mut app, from, At::Pin(to), WireStep::End);
    hold_ctrl(&mut app, false);
    app.update();
    let (wires, rev) = model_wires(&app);
    assert!(wires.contains(&format!("{d}.out -> {a}.a")) && wires.contains(&format!("{d}.out -> {b}.b")), "{wires:?}");
    assert!(!wires.iter().any(|w| w.starts_with(&format!("{c}.out"))));
    assert_eq!(rev, before.1 + 1);
    act(&mut app, Action::Undo);
    assert_eq!(model_wires(&app).0, before.0, "one undo step");
}

#[test]
fn ctrl_drag_cancel_restores() {
    let (_dir, mut app) = wires_app();
    let (c, a, b) = lerps(&mut app);
    let before = model_wires(&app);
    let (out, from, to) =
        (port(&mut app, &c, "out", true), port(&mut app, &a, "a", false), port(&mut app, &b, "b", false));
    let edge = edge_between(&mut app, out, from).unwrap();
    hold_ctrl(&mut app, true);
    wire_step(&mut app, from, At::Pin(from), WireStep::Start);
    hover_only(&mut app, Some(to));
    wire_step(&mut app, from, At::Pin(to), WireStep::Drag);
    assert_eq!(lifted(&mut app).len(), 1, "lifted mid-gesture");
    wire_step(&mut app, from, At::Pin(to), WireStep::Cancel);
    hold_ctrl(&mut app, false);
    app.update();
    assert_eq!(model_wires(&app), before, "nothing changed");
    assert!(lifted(&mut app).is_empty());
    assert_eq!(geometry_of(&app, edge).end, port_at(&app, from), "the wire is back at its pin");
}

#[test]
fn plain_drag_unchanged() {
    let (_dir, mut app) = wires_app();
    let (c, a, _) = lerps(&mut app);
    let k = add_node(&mut app, "stdlib::Constant", (-600.0, 300.0));
    lay_out_pins(&mut app);
    let (from, to) = (port(&mut app, &a, "a", false), port(&mut app, &k, "out", true));
    let out = port(&mut app, &c, "out", true);
    let edge = edge_between(&mut app, out, from).unwrap();
    wire_step(&mut app, from, At::Pin(from), WireStep::Start);
    assert!(lifted(&mut app).is_empty());
    assert_eq!(
        app.world_mut().query::<&PendingWire>().single(app.world()).unwrap().from,
        from,
        "a new wire from the pin"
    );
    assert_eq!(geometry_of(&app, edge).end, port_at(&app, from), "the old wire stays");
    hover_only(&mut app, Some(to));
    wire_step(&mut app, from, At::Pin(to), WireStep::Drag);
    wire_step(&mut app, from, At::Pin(to), WireStep::End);
    hover_only(&mut app, None);
    app.update();
    let d = app.world().resource::<Editor>().doc().unwrap();
    assert_eq!(
        d.asset.edge_into(&a, "a").map(|e| e.from.clone()),
        Some((k.clone(), "out".into())),
        "connected, replacing"
    );
}

/// A new wire dropped on a pin it may not connect to says why ("Connection refused: …") and opens no palette, also
/// with a pin it may connect to within the snap radius (Lerp `b` beside `alpha`: the pin under the pointer blocks the
/// snap, design §6.3); on empty canvas it opens the palette for the pin.
#[test]
fn plain_wire_dropped_on_refused_pin_says_why() {
    use shader_editor_app::ui::palette::Palette;
    let (_dir, mut app) = wires_app();
    let v3 = add_node(&mut app, "stdlib::Constant3Vector", (-600.0, 0.0));
    let other = add_node(&mut app, "stdlib::Constant3Vector", (-600.0, 300.0));
    let lerp = add_node(&mut app, "stdlib::Lerp", (-300.0, 0.0));
    let make = add_node(&mut app, "stdlib::MakeMaterialAttributesPbr", (-600.0, 600.0));
    lay_out_pins(&mut app);
    let before = model_wires(&app);
    let last = |app: &App| app.world().resource::<Editor>().messages.last().map(|m| m.text.clone()).unwrap_or_default();
    let drop_on = |app: &mut App, from: Entity, to: Option<Entity>| {
        wire_step(app, from, At::Pin(from), WireStep::Start);
        hover_only(app, to);
        let at = to.map_or(At::Point(Vec2::new(-450.0, -200.0)), At::Pin);
        wire_step(app, from, at, WireStep::Drag);
        let at = to.map_or(At::Point(Vec2::new(-450.0, -200.0)), At::Pin);
        wire_step(app, from, at, WireStep::End);
        hover_only(app, None);
        app.update();
    };
    // Output onto output.
    let (from, same_side) = (port(&mut app, &v3, "out", true), port(&mut app, &other, "out", true));
    drop_on(&mut app, from, Some(same_side));
    assert_eq!(last(&app), "Connection refused: an output must connect to an input");
    assert!(!app.world().resource::<Palette>().open);
    // Material attributes into a Float.
    let (from, alpha) = (port(&mut app, &make, "attributes", true), port(&mut app, &lerp, "alpha", false));
    drop_on(&mut app, from, Some(alpha));
    assert!(last(&app).starts_with("Connection refused: "), "{}", last(&app));
    assert!(!app.world().resource::<Palette>().open);
    assert_eq!(model_wires(&app), before, "nothing connected");
    // Empty canvas: the palette, linked to the pin.
    let from = port(&mut app, &v3, "out", true);
    drop_on(&mut app, from, None);
    let palette = app.world().resource::<Palette>();
    assert!(palette.open && palette.link.as_ref().is_some_and(|l| l.node == v3 && l.pin == "out"));
}

/// Design §6.3: a wire dropped 30 logical px from a compatible pin's centre, off its dot (nothing hovered), connects
/// to it: the canvas takes the nearest candidate pin within 40 px. One undo step.
#[test]
fn wire_drop_near_pin_connects() {
    let (_dir, mut app) = wires_app();
    let c = add_node(&mut app, "stdlib::Constant", (-600.0, 0.0));
    let lerp = add_node(&mut app, "stdlib::Lerp", (-300.0, 0.0));
    lay_out_pins(&mut app);
    let before = model_wires(&app);
    let (from, to) = (port(&mut app, &c, "out", true), port(&mut app, &lerp, "a", false));
    let canvas = app.world().resource::<EditorCanvas>().canvas;
    let zoom = app.world().get::<CanvasView>(canvas).unwrap().zoom;
    // 30 px left of the input, away from the node's other pins.
    let near = port_at(&app, to) - Vec2::new(30.0 / zoom, 0.0);
    wire_step(&mut app, from, At::Pin(from), WireStep::Start);
    hover_only(&mut app, None);
    wire_step(&mut app, from, At::Point(near), WireStep::Drag);
    assert_eq!(app.world_mut().query::<&WireTarget>().single(app.world()).unwrap().0, Some(to), "snaps to the pin");
    wire_step(&mut app, from, At::Point(near), WireStep::End);
    app.update();
    let (wires, rev) = model_wires(&app);
    assert!(wires.contains(&format!("{c}.out -> {lerp}.a")), "{wires:?}");
    assert_eq!(rev, before.1 + 1, "one edit");
    assert!(!app.world().resource::<shader_editor_app::ui::palette::Palette>().open, "a connection, not a drop");
    act(&mut app, Action::Undo);
    assert_eq!(model_wires(&app).0, before.0, "one undo step");
}

/// Design §6.3: every pin, reroutes included, is a port box of the physical size drawing a dot of the visual size
/// (centred, border scaled with it); a reroute node fits its two pins.
#[test]
fn pins_have_physical_and_visual_sizes() {
    use shader_editor_app::canvas::{PIN_PHYSICAL, PIN_VISUAL};
    let (_dir, mut app) = app();
    act(&mut app, Action::OpenAsset(AssetId::new(AssetType::Material, "fx/hologram")));
    act(
        &mut app,
        Action::AddNode { kind: "stdlib::Reroute".into(), pos: (-400.0, 0.0), link: None, params: Vec::new() },
    );
    let reroute = added(&app, "stdlib::Reroute", (-400.0, 0.0));
    let w = app.world_mut();
    let pins: Vec<(Entity, PinKey)> = w.query::<(Entity, &PinKey)>().iter(w).map(|(e, k)| (e, k.clone())).collect();
    assert!(pins.iter().filter(|(_, k)| k.node == reroute).count() == 2, "the reroute's pins are checked too");
    for (pin, key) in pins {
        let node = w.get::<Node>(pin).unwrap();
        assert_eq!((node.width, node.height), (px(PIN_PHYSICAL), px(PIN_PHYSICAL)), "{key:?}");
        let children = w.get::<Children>(pin).map(|c| c.to_vec()).unwrap_or_default();
        let dots: Vec<Entity> = children.into_iter().filter(|c| w.get::<PortDot>(*c).is_some()).collect();
        assert_eq!(dots.len(), 1, "{key:?}");
        let dot = w.get::<Node>(dots[0]).unwrap();
        assert_eq!((dot.width, dot.height), (px(PIN_VISUAL), px(PIN_VISUAL)), "{key:?}");
        let inset = px((PIN_PHYSICAL - PIN_VISUAL) / 2.0);
        assert_eq!((dot.left, dot.top), (inset, inset), "centred: {key:?}");
        assert_eq!(dot.border, UiRect::all(px(kit::PORT_BORDER * PIN_VISUAL / kit::PORT_SIZE)));
    }
    let node = {
        let mut q = w.query::<(&NodeKey, &Node)>();
        q.iter(w).find(|(k, _)| k.0 == reroute).map(|(_, n)| n.clone()).unwrap()
    };
    assert_eq!((node.width, node.height), (px(2.0 * PIN_PHYSICAL), px(PIN_PHYSICAL)), "the reroute fits its pins");
}

/// An output's wires lifted and dropped on empty canvas all go back to their pins.
#[test]
fn ctrl_drag_output_to_empty_restores_all_wires() {
    let (_dir, mut app) = wires_app();
    let c = add_node(&mut app, "stdlib::Constant", (-600.0, 0.0));
    let (a, b) =
        (add_node(&mut app, "stdlib::Lerp", (-300.0, 0.0)), add_node(&mut app, "stdlib::Lerp", (-300.0, 300.0)));
    wire(&mut app, (&c, "out"), (&a, "a"));
    wire(&mut app, (&c, "out"), (&b, "b"));
    lay_out_pins(&mut app);
    let before = model_wires(&app);
    let from = port(&mut app, &c, "out", true);
    let ends = [port(&mut app, &a, "a", false), port(&mut app, &b, "b", false)];
    let edges = ends.map(|to| edge_between(&mut app, from, to).unwrap());
    hold_ctrl(&mut app, true);
    wire_step(&mut app, from, At::Pin(from), WireStep::Start);
    let pointer = Vec2::new(-450.0, 600.0);
    wire_step(&mut app, from, At::Point(pointer), WireStep::Drag);
    app.update();
    for edge in edges {
        assert!((geometry_of(&app, edge).start - pointer).length() < 1e-3, "both follow the pointer");
    }
    wire_step(&mut app, from, At::Point(pointer), WireStep::End);
    hold_ctrl(&mut app, false);
    app.update();
    assert_eq!(model_wires(&app), before, "nothing changed");
    assert!(lifted(&mut app).is_empty());
    for (edge, to) in edges.into_iter().zip(ends) {
        let g = geometry_of(&app, edge);
        assert_eq!((g.start, g.end), (port_at(&app, from), port_at(&app, to)), "back at its pins");
    }
}

/// [`wires_app`] with the editor's input contexts, as in the GUI: keys reach the graph editor's context.
fn wires_keys_app() -> (tempfile::TempDir, App) {
    use bevy_enhanced_input::prelude::EnhancedInputPlugin;
    use shader_editor_app::editors::main_input::InputContextsPlugin;
    let (dir, mut app) = wires_app_with(|app| {
        app.add_plugins((
            EnhancedInputPlugin,
            bevy::input_focus::InputDispatchPlugin,
            bevy::input_focus::InputFocusPlugin,
            bevy::input_focus::tab_navigation::TabNavigationPlugin,
        ));
        app.add_plugins(InputContextsPlugin);
        app.world_mut().spawn((Window::default(), bevy::window::PrimaryWindow));
        app.finish();
    });
    app.update();
    (dir, app)
}

fn escape(app: &mut App) {
    use bevy::input::keyboard::{Key, NativeKey};
    key(app, KeyCode::Escape, Key::Unidentified(NativeKey::Unidentified), None);
}

/// Esc ends a wire drag with nothing changed (Review Focus 2): a Ctrl+drag (Ctrl still held) puts the wires back, and a
/// drop on a pin the move would succeed on afterwards does nothing; a plain wire drag is dropped too. While typing
/// (here: the palette is open) Esc is not the canvas's.
#[test]
fn escape_cancels_wire_drags() {
    let (_dir, mut app) = wires_keys_app();
    let (c, a, b) = lerps(&mut app);
    let before = model_wires(&app);
    let (out, from, to) =
        (port(&mut app, &c, "out", true), port(&mut app, &a, "a", false), port(&mut app, &b, "b", false));
    let edge = edge_between(&mut app, out, from).unwrap();
    hold_ctrl(&mut app, true);
    wire_step(&mut app, from, At::Pin(from), WireStep::Start);
    hover_only(&mut app, Some(to));
    wire_step(&mut app, from, At::Pin(to), WireStep::Drag);
    assert_eq!(lifted(&mut app).len(), 1);

    app.world_mut().resource_mut::<shader_editor_app::ui::palette::Palette>().open = true;
    escape(&mut app);
    assert_eq!(lifted(&mut app).len(), 1, "typing keeps Esc");
    app.world_mut().resource_mut::<shader_editor_app::ui::palette::Palette>().open = false;
    app.update();

    escape(&mut app);
    assert!(lifted(&mut app).is_empty(), "Esc puts the wires back");
    wire_step(&mut app, from, At::Pin(to), WireStep::End);
    hold_ctrl(&mut app, false);
    hover_only(&mut app, None);
    app.update();
    assert_eq!(model_wires(&app), before, "the model and its revision are unchanged");
    assert_eq!(geometry_of(&app, edge).end, port_at(&app, from), "the wire is back at its pin");

    // A plain wire drag.
    // Without Esc, dropping on `b.out` would connect it to `a.a` (replacing `c.out`).
    let k = port(&mut app, &b, "out", true);
    wire_step(&mut app, from, At::Pin(from), WireStep::Start);
    assert_eq!(pending_wires(&mut app), 1);
    escape(&mut app);
    assert_eq!(pending_wires(&mut app), 0, "Esc drops the new wire");
    hover_only(&mut app, Some(k));
    wire_step(&mut app, from, At::Pin(k), WireStep::End);
    hover_only(&mut app, None);
    app.update();
    assert_eq!(model_wires(&app), before);
}

/// [`typing_app`] with the headless buttons and menu logic (`bevy_ui_widgets`: focus on open, close on focus loss,
/// keys), bevy_picking's pointer pipeline ([`press_on`]) and the observer that turns an activated widget's
/// `OnActivate` into an action. Every entity counts as visible: a menu focuses only visible items, and visibility is
/// not propagated without a renderer.
fn menu_app() -> (tempfile::TempDir, App) {
    let (dir, mut app) = typing_app_with(|app| {
        let visible =
            |mut q: Query<&mut InheritedVisibility>| q.iter_mut().for_each(|mut v| *v = InheritedVisibility::VISIBLE);
        app.add_plugins((
            bevy::ui_widgets::ButtonPlugin,
            bevy::ui_widgets::MenuPlugin,
            bevy::picking::PickingPlugin,
            bevy::picking::InteractionPlugin,
        ))
        .add_systems(PostUpdate, visible.before(bevy::ui_widgets::MenuFocusSystem))
        .add_observer(shader_editor_app::ui::activate_action);
    });
    app.world_mut().spawn(bevy::picking::pointer::PointerId::Mouse);
    app.update();
    (dir, app)
}

/// A left click through bevy_picking: the pointer is over `stack` (topmost first, as a picking backend reports the
/// UI nodes under it), pressed and released. Picking delivers the press to the topmost entity and to the ones below
/// it until one blocks them (`Pickable::should_block_lower`, on by default for UI nodes).
fn press_on(app: &mut App, stack: &[Entity]) {
    use bevy::camera::NormalizedRenderTarget;
    use bevy::picking::backend::{HitData, PointerHits};
    use bevy::picking::pointer::{Location, PointerAction, PointerButton, PointerId, PointerInput};
    let target = NormalizedRenderTarget::None { width: 800, height: 600 };
    let location = Location { target, position: Vec2::new(5.0, 5.0) };
    for action in [PointerAction::Press(PointerButton::Primary), PointerAction::Release(PointerButton::Primary)] {
        let w = app.world_mut();
        let hits = stack.iter().enumerate().map(|(i, e)| (*e, HitData::new(Entity::PLACEHOLDER, i as f32, None, None)));
        w.write_message(PointerHits::new(PointerId::Mouse, hits.collect(), 0.0));
        w.write_message(PointerInput::new(PointerId::Mouse, location.clone(), action));
        app.update();
    }
    app.update();
}

/// A plain Feathers button somewhere under the menus, and a counter of its `Activate`s.
fn target_button(app: &mut App) -> (Entity, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    let hits = Arc::new(AtomicUsize::new(0));
    let counter = hits.clone();
    let e = app.world_mut().spawn_scene(shader_editor_app::ui::button("Under", Action::RegenAll, false)).unwrap().id();
    app.world_mut().entity_mut(e).observe(move |_: On<bevy::ui_widgets::Activate>| {
        counter.fetch_add(1, Ordering::SeqCst);
    });
    app.update();
    (e, hits)
}

/// The full-window barrier of the open menu (context or main).
fn barrier(app: &mut App) -> Entity {
    use shader_editor_app::ui::dialog::MENU_BARRIER_Z;
    let w = app.world_mut();
    let found: Vec<Entity> =
        w.query::<(Entity, &GlobalZIndex)>().iter(w).filter(|(_, z)| z.0 == MENU_BARRIER_Z).map(|(e, _)| e).collect();
    assert_eq!(found.len(), 1, "one barrier: {found:?}");
    found[0]
}

fn context_menus(app: &mut App) -> usize {
    let w = app.world_mut();
    w.query_filtered::<(), With<shader_editor_app::ui::dialog::MenuRoot>>().iter(w).count()
}

/// The items of the open menus with their `InteractionDisabled`, top to bottom.
fn menu_items(app: &mut App) -> Vec<(Entity, bool)> {
    use bevy::feathers::controls::FeathersMenuItem;
    use bevy::ui::InteractionDisabled;
    use bevy::ui_widgets::MenuPopup;
    let w = app.world_mut();
    let rows: Vec<Entity> =
        w.query_filtered::<&Children, With<MenuPopup>>().iter(w).flat_map(|c| c.iter().collect::<Vec<_>>()).collect();
    let mut items = w.query_filtered::<Has<InteractionDisabled>, With<FeathersMenuItem>>();
    rows.into_iter().filter_map(|e| items.get(w, e).ok().map(|d| (e, d))).collect()
}

/// A context menu (design §3 audit): a Feathers menu popup at the cursor — items, separators as dividers, a disabled
/// item — that takes the focus and the keyboard (no hotkeys) while open. A click outside it lands on its barrier: it
/// closes the menu and does nothing else.
#[test]
fn context_menu_is_a_feathers_menu_that_only_closes_on_outside_click() {
    use bevy::feathers::controls::FeathersMenuDivider;
    use bevy::input_focus::InputFocus;
    use shader_editor_app::editors::main_input::KeyboardGate;
    use shader_editor_app::ui::dialog::{MenuEntry, open_menu_entries};
    use shader_editor_app::ui::icons::IconId;

    let (_dir, mut app) = menu_app();
    let entries = vec![
        MenuEntry { icon: Some(IconId::Save), label: "Save\tCtrl+S".into(), action: Action::Save, disabled: None },
        MenuEntry { icon: None, label: String::new(), action: Action::None, disabled: None },
        MenuEntry { icon: None, label: "Regenerate".into(), action: Action::RegenAll, disabled: Some("why".into()) },
    ];
    let mut commands = app.world_mut().commands();
    open_menu_entries(&mut commands, Vec2::new(40.0, 50.0), entries.clone());
    app.update();
    app.update();
    assert_eq!(context_menus(&mut app), 1);
    let items = menu_items(&mut app);
    assert_eq!(items.iter().map(|(_, d)| *d).collect::<Vec<_>>(), [false, true], "an item, a disabled one");
    let w = app.world_mut();
    assert_eq!(w.query_filtered::<(), With<FeathersMenuDivider>>().iter(w).count(), 1, "the separator");
    assert_eq!(app.world().resource::<InputFocus>().get(), Some(items[0].0), "the first item has the focus");
    assert!(app.world().resource::<KeyboardGate>().typing, "an open menu takes the keyboard");

    // A second menu replaces the first.
    let mut commands = app.world_mut().commands();
    open_menu_entries(&mut commands, Vec2::new(80.0, 90.0), entries);
    app.update();
    app.update();
    assert_eq!(context_menus(&mut app), 1, "one context menu at a time");

    // A click inside the popup that hits no item (a divider) keeps it open.
    let (under, activated) = target_button(&mut app);
    let barrier = barrier(&mut app);
    let divider = {
        let w = app.world_mut();
        w.query_filtered::<Entity, With<FeathersMenuDivider>>().single(w).unwrap()
    };
    press_on(&mut app, &[divider, barrier, under]);
    assert_eq!(context_menus(&mut app), 1, "a click on a divider keeps the menu");

    // A click outside the popup lands on the barrier: the menu closes, and the button under it gets nothing.
    press_on(&mut app, &[barrier, under]);
    assert_eq!(context_menus(&mut app), 0, "closed by a click outside");
    assert_eq!(activated.load(std::sync::atomic::Ordering::SeqCst), 0, "the click went nowhere else");
    assert_eq!(app.world().resource::<InputFocus>().get(), None);
    assert!(!app.world().resource::<KeyboardGate>().typing, "the hotkeys are back");
    // The same click without a menu reaches the button: the harness delivers presses.
    press_on(&mut app, &[under]);
    assert_eq!(activated.load(std::sync::atomic::Ordering::SeqCst), 1, "a click with no menu open");
}

/// Clicking a context menu item pushes its action and closes the menu; a disabled item does neither. Esc closes it.
#[test]
fn context_menu_item_click_and_esc() {
    use bevy::input::keyboard::{Key, NativeKey};
    use shader_editor_app::ui::dialog::{MenuEntry, open_menu_entries};

    let (_dir, mut app) = menu_app();
    let open = |app: &mut App| {
        let entries = vec![
            MenuEntry { icon: None, label: "Off".into(), action: Action::RegenAll, disabled: Some("why".into()) },
            MenuEntry { icon: None, label: "Clear".into(), action: Action::ClearMessages, disabled: None },
        ];
        let mut commands = app.world_mut().commands();
        open_menu_entries(&mut commands, Vec2::new(40.0, 50.0), entries);
        app.update();
        app.update();
        std::mem::take(&mut app.world_mut().resource_mut::<Actions>().0);
    };
    let pushed = |app: &mut App| std::mem::take(&mut app.world_mut().resource_mut::<Actions>().0);

    open(&mut app);
    let items = menu_items(&mut app);
    assert_eq!(items.iter().map(|(_, d)| *d).collect::<Vec<_>>(), [true, false]);
    click(&mut app, items[0].0);
    assert!(pushed(&mut app).is_empty(), "a disabled item pushes nothing");
    app.update();
    assert_eq!(context_menus(&mut app), 1, "and keeps the menu open");
    click(&mut app, items[1].0);
    assert!(matches!(pushed(&mut app)[..], [Action::ClearMessages]), "an item pushes its action");
    app.update();
    assert_eq!(context_menus(&mut app), 0, "and closes the menu");

    open(&mut app);
    key(&mut app, KeyCode::Escape, Key::Unidentified(NativeKey::Unidentified), None);
    assert_eq!(context_menus(&mut app), 0, "Esc closes it");
    assert!(pushed(&mut app).is_empty(), "without an action");
}

/// The icons of each item of the open or closed menus, top to bottom.
fn menu_item_icons(app: &mut App) -> Vec<Vec<shader_editor_app::ui::icons::IconId>> {
    use shader_editor_app::ui::icons::UiIcon;
    let items: Vec<Entity> = menu_items(app).into_iter().map(|(e, _)| e).collect();
    let w = app.world_mut();
    let mut children = w.query::<&Children>();
    let mut icons = w.query::<&UiIcon>();
    items
        .into_iter()
        .map(|item| {
            let mut found = Vec::new();
            let mut stack = vec![item];
            while let Some(e) = stack.pop() {
                // An icon with an accent layer has a second `UiIcon` of the same id.
                if let Ok(i) = icons.get(w, e)
                    && !found.contains(&i.id)
                {
                    found.push(i.id);
                }
                if let Ok(c) = children.get(w, e) {
                    stack.extend(c.iter());
                }
            }
            found
        })
        .collect()
}

/// The main menu (design §3 audit, §4): a `FeathersMenu` whose button toggles its popup under it; the View items'
/// check marks follow the config; an item pushes its action and closes the menu, giving the focus back to its button.
#[test]
fn main_menu_is_a_feathers_menu_with_check_marks() {
    use bevy::input_focus::InputFocus;
    use bevy::ui_widgets::{MenuButton, MenuPopup};
    use shader_editor_app::editors::main_input::KeyboardGate;
    use shader_editor_app::ui::icons::IconId;
    use shader_editor_app::ui::{MainMenu, fill_main_menus, main_menu};

    let (_dir, mut app) = menu_app();
    app.add_systems(Update, (fill_main_menus, shader_editor_app::ui::main_menu_barrier));
    app.world_mut().spawn_scene(main_menu(MainMenu::View)).unwrap();
    app.update();
    app.update();
    let popup_shown = |app: &mut App| {
        let w = app.world_mut();
        *w.query_filtered::<&Visibility, With<MenuPopup>>().single(w).unwrap() == Visibility::Visible
    };
    let button = {
        let w = app.world_mut();
        w.query_filtered::<Entity, With<MenuButton>>().single(w).unwrap()
    };
    assert!(!popup_shown(&mut app), "closed");
    let check = || vec![IconId::Check];
    assert_eq!(menu_item_icons(&mut app), [vec![], check(), check(), check()], "Zen off, every panel shown");

    click(&mut app, button);
    app.update();
    app.update();
    assert!(popup_shown(&mut app), "the button opens it");
    let items = menu_items(&mut app);
    assert_eq!(app.world().resource::<InputFocus>().get(), Some(items[0].0), "its first item has the focus");
    assert!(app.world().resource::<KeyboardGate>().typing, "an open menu takes the keyboard");
    click(&mut app, button);
    app.update();
    app.update();
    assert!(!popup_shown(&mut app), "the button closes it again");

    click(&mut app, button);
    app.update();
    app.update();
    click(&mut app, items[0].0);
    let pushed = std::mem::take(&mut app.world_mut().resource_mut::<Actions>().0);
    assert!(matches!(pushed[..], [Action::ToggleZen]), "Zen mode pushes its toggle: {pushed:?}");
    app.update();
    app.update();
    assert!(!popup_shown(&mut app), "an item closes the menu");
    // Bevy's default (design §2): the item hands the focus back to its menu button (`MenuAction::FocusRoot`).
    assert_eq!(app.world().resource::<InputFocus>().get(), Some(button), "the focus back on the menu button");

    // A click outside an open main menu only closes it; the barrier goes with it.
    click(&mut app, button);
    app.update();
    app.update();
    assert!(popup_shown(&mut app));
    let (under, activated) = target_button(&mut app);
    let barrier = barrier(&mut app);
    press_on(&mut app, &[barrier, under]);
    assert!(!popup_shown(&mut app), "a click outside closes it");
    assert_eq!(activated.load(std::sync::atomic::Ordering::SeqCst), 0, "and reaches nothing under the barrier");
    let w = app.world_mut();
    assert_eq!(w.query_filtered::<(), With<shader_editor_app::ui::MainMenuBarrier>>().iter(w).count(), 0);

    app.world_mut().resource_mut::<Editor>().config.show_log = false;
    app.update();
    assert_eq!(menu_item_icons(&mut app), [vec![], check(), vec![], check()], "the check mark follows the config");
}

/// The texts of each menu item (its label, then its hotkey), in order.
fn menu_item_texts(app: &mut App) -> Vec<Vec<String>> {
    let items: Vec<Entity> = menu_items(app).into_iter().map(|(e, _)| e).collect();
    let w = app.world_mut();
    let mut children = w.query::<&Children>();
    let mut texts = w.query::<&Text>();
    items
        .into_iter()
        .map(|item| {
            let mut found = Vec::new();
            let mut stack = vec![item];
            while let Some(e) = stack.pop() {
                if let Ok(t) = texts.get(w, e)
                    && !t.0.is_empty()
                {
                    found.push(t.0.clone());
                }
                if let Ok(c) = children.get(w, e) {
                    stack.extend(c.iter().rev());
                }
            }
            found
        })
        .collect()
}

/// Design §3: the View menu lists Zen mode, a separator, then the three panels, each with its hotkey; the check marks
/// follow the four flags, also a panel's own flag while Zen mode hides it.
#[test]
fn view_menu_marks_follow_flags() {
    use bevy::feathers::controls::FeathersMenuDivider;
    use bevy::ui_widgets::MenuPopup;
    use shader_editor_app::ui::icons::IconId;
    use shader_editor_app::ui::{MainMenu, fill_main_menus, main_menu};

    let (_dir, mut app) = menu_app();
    app.add_systems(Update, fill_main_menus);
    app.world_mut().spawn_scene(main_menu(MainMenu::View)).unwrap();
    app.update();
    app.update();
    let texts = menu_item_texts(&mut app);
    assert_eq!(
        texts,
        [
            vec!["Zen mode".to_string(), "`".into()],
            vec!["Show project".into(), "Ctrl+1".into()],
            vec!["Show console".into(), "Ctrl+2".into()],
            vec!["Show properties".into(), "Ctrl+3".into()],
        ]
    );
    // The separator sits between Zen mode and the panels.
    let rows: Vec<(bool, bool)> = {
        let w = app.world_mut();
        let popup = w.query_filtered::<&Children, With<MenuPopup>>().single(w).unwrap().to_vec();
        let mut kind = w.query::<(Has<FeathersMenuDivider>, Has<bevy::feathers::controls::FeathersMenuItem>)>();
        popup.into_iter().filter_map(|e| kind.get(w, e).ok()).filter(|(d, i)| *d || *i).collect()
    };
    assert_eq!(rows, [(false, true), (true, false), (false, true), (false, true), (false, true)]);

    let check = || vec![IconId::Check];
    let set = |app: &mut App, f: fn(&mut shader_editor_app::config::UserConfig)| {
        f(&mut app.world_mut().resource_mut::<Editor>().config);
        app.update();
        app.update();
        menu_item_icons(app)
    };
    assert_eq!(set(&mut app, |_| {}), [vec![], check(), check(), check()]);
    assert_eq!(set(&mut app, |c| c.show_left_sidebar = false), [vec![], vec![], check(), check()]);
    assert_eq!(set(&mut app, |c| c.zen_mode = true), [check(), vec![], check(), check()]);
    assert_eq!(set(&mut app, |c| c.show_properties = false), [check(), vec![], check(), vec![]], "in Zen too");
    assert_eq!(set(&mut app, |c| c.show_log = false), [check(), vec![], vec![], vec![]]);
}

/// Design §3: Zen mode hides the whole console/logs, its header included (and from Tab: `Visibility::Hidden`); off,
/// the console comes back as its flag says.
#[test]
fn zen_hides_whole_console() {
    use shader_editor_app::ui::log::{LogBody, LogPane, LogPlugin, console};

    let (_dir, mut app) = app();
    app.add_plugins(LogPlugin);
    app.world_mut().spawn_scene(console()).unwrap();
    app.update();
    // (display and visibility of the console, display of the pane body around `LogBody`)
    let state = |app: &mut App| {
        let w = app.world_mut();
        let (node, vis) = w.query_filtered::<(&Node, &Visibility), With<LogPane>>().single(w).unwrap();
        let (display, vis) = (node.display, *vis);
        let body = w.query_filtered::<&ChildOf, With<LogBody>>().single(w).unwrap().parent();
        (display, vis, w.get::<Node>(body).unwrap().display)
    };
    let shown = (Display::Flex, Visibility::Inherited, Display::Flex);
    assert_eq!(state(&mut app), shown);
    act(&mut app, Action::ToggleZen);
    assert_eq!(state(&mut app).0, Display::None, "Zen: no console, not even its header");
    assert_eq!(state(&mut app).1, Visibility::Hidden);
    act(&mut app, Action::ToggleLog);
    assert_eq!(state(&mut app).0, Display::None, "Ctrl+2 in Zen: still hidden");
    act(&mut app, Action::ToggleZen);
    assert_eq!(state(&mut app), (Display::Flex, Visibility::Inherited, Display::None), "Zen off: the header only");
    act(&mut app, Action::ToggleLog);
    assert_eq!(state(&mut app), shown);
    assert_eq!(idle_node_changes::<shader_editor_app::ui::log::LogPane>(&mut app), 0, "no writes when idle");
}

/// How many `M` entities had their `Node` changed during two idle updates (after one to settle): the layout systems
/// write only on change, so a steady state is 0. Compares change ticks: a `Changed` query from the test would not see
/// an update's writes once `App::update` has cleared the trackers.
fn idle_node_changes<M: Component>(app: &mut App) -> usize {
    let ticks = |app: &mut App| {
        let w = app.world_mut();
        w.query_filtered::<Ref<Node>, With<M>>().iter(w).map(|n| n.last_changed()).collect::<Vec<_>>()
    };
    app.update();
    let before = ticks(app);
    app.update();
    app.update();
    before.iter().zip(ticks(app)).filter(|(a, b)| **a != *b).count()
}

/// Design §3: Ctrl+1 off, the left sidebar takes no space (and Tab skips it); Zen mode hides it and the right sidebar
/// whatever their flags say, and Zen off shows exactly the ones whose flags are on.
#[test]
fn hidden_left_sidebar_takes_no_space() {
    use shader_editor_app::ui::areas::{AreasPlugin, RightSidebar, right_sidebar};
    use shader_editor_app::ui::{LeftSidebar, left_sidebar, show_left_sidebar};

    let (_dir, mut app) = app();
    app.add_plugins(AreasPlugin).add_systems(Update, show_left_sidebar);
    app.world_mut().spawn_scene(left_sidebar()).unwrap();
    app.world_mut().spawn_scene(right_sidebar()).unwrap();
    act(&mut app, Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet")));
    let state = |app: &mut App| {
        let w = app.world_mut();
        let (left, vis) = w.query_filtered::<(&Node, &Visibility), With<LeftSidebar>>().single(w).unwrap();
        let (left, vis) = (left.display, *vis);
        let right = w.query_filtered::<&Node, With<RightSidebar>>().single(w).unwrap().display;
        (left, vis, right)
    };
    let (flex, none) = (Display::Flex, Display::None);
    assert_eq!(state(&mut app), (flex, Visibility::Inherited, flex));
    act(&mut app, Action::ToggleLeftSidebar);
    assert_eq!(state(&mut app), (none, Visibility::Hidden, flex), "Ctrl+1: no left sidebar");
    act(&mut app, Action::ToggleLeftSidebar);
    assert_eq!(state(&mut app), (flex, Visibility::Inherited, flex));
    act(&mut app, Action::ToggleZen);
    assert_eq!(state(&mut app), (none, Visibility::Hidden, none), "Zen: both sidebars hidden");
    act(&mut app, Action::ToggleLeftSidebar);
    assert_eq!(state(&mut app).0, none, "Ctrl+1 in Zen: still hidden");
    act(&mut app, Action::ToggleZen);
    assert_eq!(state(&mut app), (none, Visibility::Hidden, flex), "Zen off: the left flag is off now");
    assert_eq!(idle_node_changes::<LeftSidebar>(&mut app), 0, "no writes when idle");
    act(&mut app, Action::ToggleLeftSidebar);
    assert_eq!(idle_node_changes::<LeftSidebar>(&mut app), 0, "shown: no writes when idle either");
}

/// Design §3: Ctrl+1 is a View hotkey, not the "hold 1 and click the canvas" Constant drop; plain 1 still drops one.
#[test]
fn ctrl_1_does_not_arm_constant_drop() {
    let (_dir, mut app) = gestures_app();
    act(&mut app, Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet")));
    let canvas = app.world().resource::<EditorCanvas>().canvas;
    app.world_mut().entity_mut(canvas).insert(ComputedNode {
        size: Vec2::new(800.0, 600.0),
        inverse_scale_factor: 1.0,
        ..default()
    });
    let constants = |app: &App| {
        let d = app.world().resource::<Editor>().doc().unwrap();
        d.asset.nodes.values().filter(|n| n.kind == "stdlib::Constant").count()
    };
    let n = constants(&app);
    let drop = |app: &mut App, keys: &[KeyCode]| {
        let mut input = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
        keys.iter().for_each(|k| input.press(*k));
        app.update();
        click(app, canvas);
        app.update();
        app.update();
        app.world_mut().resource_mut::<ButtonInput<KeyCode>>().release_all();
        app.update();
    };
    drop(&mut app, &[KeyCode::ControlLeft, KeyCode::Digit1]);
    assert_eq!(constants(&app), n, "Ctrl+1 + click: no Constant");
    drop(&mut app, &[KeyCode::SuperLeft, KeyCode::Digit1]);
    assert_eq!(constants(&app), n, "Super+1 + click: no Constant");
    drop(&mut app, &[KeyCode::Digit1]);
    assert_eq!(constants(&app), n + 1, "1 + click: a Constant");
}

/// One frame of bevy_picking: the pointer is over `stack` (topmost first) and, with `action`, presses or releases.
fn pointer_frame(app: &mut App, stack: &[Entity], action: Option<bevy::picking::pointer::PointerAction>) {
    use bevy::camera::NormalizedRenderTarget;
    use bevy::picking::backend::{HitData, PointerHits};
    use bevy::picking::pointer::{Location, PointerId, PointerInput};
    let target = NormalizedRenderTarget::None { width: 800, height: 600 };
    let location = Location { target, position: Vec2::new(5.0, 5.0) };
    let w = app.world_mut();
    let hits = stack.iter().enumerate().map(|(i, e)| (*e, HitData::new(Entity::PLACEHOLDER, i as f32, None, None)));
    w.write_message(PointerHits::new(PointerId::Mouse, hits.collect(), 0.0));
    if let Some(action) = action {
        w.write_message(PointerInput::new(PointerId::Mouse, location, action));
    }
    app.update();
}

/// A left click on a menu_bar menu button as in the GUI: the press opens the menu, so the menu's barrier is over the
/// button from the next frame on and the release lands on it.
fn click_menu_button(app: &mut App, button: Entity) {
    use bevy::picking::pointer::{PointerAction, PointerButton};
    use shader_editor_app::ui::MainMenuBarrier;
    pointer_frame(app, &[button], Some(PointerAction::Press(PointerButton::Primary)));
    let w = app.world_mut();
    let mut stack: Vec<Entity> = w.query_filtered::<Entity, With<MainMenuBarrier>>().iter(w).collect();
    stack.push(button);
    pointer_frame(app, &stack, None);
    pointer_frame(app, &stack, Some(PointerAction::Release(PointerButton::Primary)));
    pointer_frame(app, &stack, None);
    app.update();
}

/// Design §1: a menu_bar menu button looks active (Feathers' `Pressed`) while its menu is open and returns to normal
/// whenever the menu closes — by an outside click on the barrier, Esc, an item or opening another menu — and a click
/// opens it again.
#[test]
fn main_menu_button_resets_on_every_close() {
    use bevy::ui::Pressed;
    use bevy::ui_widgets::MenuButton;
    use shader_editor_app::ui::{MainMenu, MainMenuPopup, fill_main_menus, main_menu, main_menu_barrier};

    let (_dir, mut app) = menu_app();
    app.add_systems(Update, (fill_main_menus, main_menu_barrier));
    for m in [MainMenu::File, MainMenu::View] {
        app.world_mut().spawn_scene(main_menu(m)).unwrap();
    }
    app.update();
    app.update();
    // Each menu's button and popup, by the popup's menu.
    let parts = |app: &mut App, menu: MainMenu| -> (Entity, Entity) {
        let w = app.world_mut();
        let popup = w.query::<(Entity, &MainMenuPopup)>().iter(w).find(|(_, p)| p.0 == menu).unwrap().0;
        let root = w.get::<ChildOf>(popup).unwrap().parent();
        let button = w.get::<Children>(root).unwrap().iter().find(|c| w.get::<MenuButton>(*c).is_some()).unwrap();
        (button, popup)
    };
    let (file, file_popup) = parts(&mut app, MainMenu::File);
    let (view, view_popup) = parts(&mut app, MainMenu::View);
    // (open, looks active)
    let state = |app: &App, (button, popup): (Entity, Entity)| {
        let w = app.world();
        (*w.get::<Visibility>(popup).unwrap() == Visibility::Visible, w.get::<Pressed>(button).is_some())
    };
    let closed = (false, false);
    let (file_menu, view_menu) = ((file, file_popup), (view, view_popup));
    assert_eq!(state(&app, file_menu), closed);

    // An outside click on the barrier.
    click_menu_button(&mut app, file);
    assert_eq!(state(&app, file_menu), (true, true), "open and active");
    let (under, _) = target_button(&mut app);
    let b = barrier(&mut app);
    press_on(&mut app, &[b, under]);
    assert_eq!(state(&app, file_menu), closed, "an outside click");

    // Esc.
    click_menu_button(&mut app, file);
    assert_eq!(state(&app, file_menu), (true, true), "a click opens it again");
    escape(&mut app);
    assert_eq!(state(&app, file_menu), closed, "Esc");

    // An item.
    click_menu_button(&mut app, view);
    assert_eq!(state(&app, view_menu), (true, true));
    let item =
        menu_items(&mut app).into_iter().find(|(e, _)| app.world().get::<ChildOf>(*e).unwrap().parent() == view_popup);
    let b = barrier(&mut app);
    press_on(&mut app, &[item.unwrap().0, b]);
    std::mem::take(&mut app.world_mut().resource_mut::<Actions>().0);
    assert_eq!(state(&app, view_menu), closed, "an item");

    // Opening another menu: the press on the other button lands on the barrier and closes the open menu; the next
    // click opens the other one.
    click_menu_button(&mut app, file);
    let b = barrier(&mut app);
    press_on(&mut app, &[b, view]);
    assert_eq!(state(&app, file_menu), closed, "another menu's button");
    click_menu_button(&mut app, view);
    assert_eq!((state(&app, file_menu), state(&app, view_menu)), (closed, (true, true)), "the other menu opens");
    escape(&mut app);
    assert_eq!(state(&app, view_menu), closed);
}

/// The subpane titles under `root`, in display order (a Feathers `subpane_header` is told by its theme token).
fn subpane_titles(app: &mut App, root: Entity) -> Vec<String> {
    use bevy::feathers::theme::ThemeBackgroundColor;
    use bevy::feathers::tokens::SUBPANE_HEADER_BG;

    fn texts(w: &World, e: Entity, out: &mut Vec<String>) {
        if let Some(t) = w.get::<Text>(e) {
            out.push(t.0.clone());
        }
        for c in w.get::<Children>(e).into_iter().flatten() {
            texts(w, *c, out);
        }
    }
    fn walk(w: &World, e: Entity, out: &mut Vec<String>) {
        if w.get::<ThemeBackgroundColor>(e).is_some_and(|c| c.0 == SUBPANE_HEADER_BG) {
            let mut title = Vec::new();
            texts(w, e, &mut title);
            out.push(title.concat());
            return;
        }
        for c in w.get::<Children>(e).into_iter().flatten() {
            walk(w, *c, out);
        }
    }
    let mut out = Vec::new();
    walk(app.world(), root, &mut out);
    out
}

/// Design §5: with no selection, a material's and a function's right sidebar has no "Info" subpane and ends with
/// "Hotkeys", then "Description".
#[test]
fn graph_sidebar_has_no_info_and_description_is_last() {
    use shader_editor_app::ui::areas::{AreasPlugin, RightSidebar, right_sidebar};
    use shader_editor_app::ui::props::PropsPlugin;

    let (_dir, mut app) = app();
    app.add_plugins((AreasPlugin, PropsPlugin));
    app.world_mut().spawn_scene(right_sidebar()).unwrap();
    let sidebar = app.world_mut().query_filtered::<Entity, With<RightSidebar>>().single(app.world()).unwrap();
    for id in [AssetId::new(AssetType::Material, "env/rock_wet"), AssetId::new(AssetType::Function, "common/wet_mask")]
    {
        act(&mut app, Action::OpenAsset(id.clone()));
        act(&mut app, Action::Select(Default::default()));
        let titles = subpane_titles(&mut app, sidebar);
        assert!(titles.ends_with(&["Hotkeys".to_string(), "Description".to_string()]), "{id:?}: {titles:?}");
        assert!(!titles.iter().any(|t| t == "Info"), "{id:?}: {titles:?}");
    }
}

/// Design §6.1: the editor topbar's Comment button (`AddComment(None)`, also hold C off the canvas) puts a frame
/// around no node at the visible canvas centre, snapped to the grid.
#[test]
fn comment_button_adds_frame_at_view_centre() {
    use shader_editor_app::ui::areas::AreasPlugin;

    let (_dir, mut app) = app();
    app.add_plugins(AreasPlugin);
    act(&mut app, Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet")));
    act(&mut app, Action::Select(Default::default()));
    let canvas = app.world().resource::<EditorCanvas>().canvas;
    {
        let mut view = app.world_mut().get_mut::<CanvasView>(canvas).unwrap();
        view.pan = Vec2::new(-1003.0, 497.0);
        view.zoom = 2.0;
    }
    // A laid-out canvas of 840x620 logical px (physical 1680x1240 at scale factor 2); no UI layout runs here.
    let laid_out =
        ComputedNode { size: Vec2::new(1680.0, 1240.0), inverse_scale_factor: 0.5, ..ComputedNode::default() };
    app.world_mut().entity_mut(canvas).insert(laid_out);
    // (840, 620) / 2 = (420, 310) canvas px; minus the pan, over the zoom.
    let centre = Vec2::new((420.0 + 1003.0) / 2.0, (310.0 - 497.0) / 2.0);
    let before: Vec<String> =
        app.world().resource::<Editor>().doc().unwrap().asset.comments.iter().map(|f| f.id.clone()).collect();
    act(&mut app, Action::AddComment(None));
    let d = app.world().resource::<Editor>().doc().unwrap();
    let f = d.asset.comments.iter().find(|f| !before.contains(&f.id)).expect("a new frame");
    let snapped = shader_editor_app::doc::snap((centre.x, centre.y));
    assert_eq!(snapped, (710.0, -90.0));
    assert_eq!(f.rect, (snapped.0, snapped.1, 400.0, 300.0));
    assert!(f.nodes.is_empty());
}
