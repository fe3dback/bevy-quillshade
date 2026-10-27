//! Pointer interaction, headless: pointer events are triggered directly, as
//! `bevy_picking` would.

// Test helpers may panic: a panic is a failed test.
#![allow(clippy::disallowed_methods, clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use bevy::camera::NormalizedRenderTarget;
use bevy::ecs::entity::EntityHashSet;
use bevy::input::gestures::PinchGesture;
use bevy::input_focus::tab_navigation::TabIndex;
use bevy::picking::backend::HitData;
use bevy::picking::hover::HoverMap;
use bevy::picking::pointer::{Location, PointerId, PointerLocation};
use bevy::prelude::*;
use bevy::ui::{ComputedNode, InteractionDisabled, Selected, UiGlobalTransform, UiScale};
use bevy_noodle::prelude::*;
use bevy_noodle::{DragProgress, EdgesDropped, MoveCheck, MovingEdges, PortAnchor, WireCandidates, WireTarget};

const NUM: PortType = PortType::named("num");

fn app() -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, bevy::input::InputPlugin, NoodlePlugins));
    // Pointer events bubble through `PointerTraversal`, which reads `Window`.
    app.world_mut().register_component::<Window>();
    app
}

fn graph(world: &mut World) -> (Entity, Entity) {
    let canvas = world.spawn((NodeCanvas, CanvasInteraction::default(), laid_out())).id();
    world.flush();
    (canvas, world.get::<Children>(canvas).unwrap()[0])
}

/// A UI node as layout leaves it: 400 by 300 pixels at the window's origin.
fn laid_out() -> (Node, ComputedNode) {
    let size = Vec2::new(400.0, 300.0);
    let computed = ComputedNode { size, ..default() };
    (Node::default(), computed)
}

/// One node per port in `content`; returns the ports.
fn one_node_each<const N: usize>(w: &mut World, content: Entity, ports: [Port; N]) -> [Entity; N] {
    ports.map(|port| {
        let node = w.spawn((GraphNode, Node::default(), ChildOf(content))).id();
        w.spawn((port, Node::default(), ChildOf(node))).id()
    })
}

/// Whether a dragged wire may connect to `port`.
fn candidate(w: &mut World, port: Entity) -> bool {
    w.query::<&WireCandidates>().iter(w).any(|c| c.contains(&port))
}

fn hit() -> HitData {
    HitData::new(Entity::PLACEHOLDER, 0.0, None, None)
}

/// A pointer event's own fields; `pointer` adds the target and the pointer.
trait Fields {
    fn trigger(self, world: &mut World, entity: Entity, pointer: Pointer);
}

macro_rules! fields {
    ($name:ident => $event:ident { $($field:ident: $ty:ty),* }) => {
        struct $name { $($field: $ty),* }
        impl Fields for $name {
            fn trigger(self, world: &mut World, entity: Entity, pointer: Pointer) {
                world.trigger($event { entity, pointer, $($field: self.$field),* });
            }
        }
    };
}

fields!(Press => PointerPress { button: PointerButton, hit: HitData, count: u8 });
fields!(DragStart => PointerDragStart { button: PointerButton, hit: HitData });
fields!(Drag => PointerDrag { button: PointerButton, distance: Vec2, delta: Vec2 });
fields!(DragEnd => PointerDragEnd { button: PointerButton, distance: Vec2 });
fields!(Cancel => PointerCancel { hit: HitData });

/// Triggers a pointer event on `target` (bubbling up), then applies its edits.
fn pointer(world: &mut World, target: Entity, fields: impl Fields) {
    pointer_at(world, target, Vec2::ZERO, fields);
}

/// [`pointer`] with the pointer at window `position`.
fn pointer_at(world: &mut World, target: Entity, position: Vec2, fields: impl Fields) {
    let target_none = NormalizedRenderTarget::None { width: 1, height: 1 };
    let location = Location { target: target_none, position };
    fields.trigger(world, target, Pointer::new(PointerId::Mouse, location));
    world.flush();
}

fn drag(world: &mut World, target: Entity, by: Vec2) {
    let button = PointerButton::Primary;
    pointer(world, target, DragStart { button, hit: hit() });
    pointer(world, target, Drag { button, distance: by, delta: by });
    pointer(world, target, DragEnd { button, distance: by });
}

#[test]
fn nodes_with_a_drag_handle_move_only_from_it() {
    let mut app = app();
    let w = app.world_mut();
    let (_, content) = graph(w);
    let node = w.spawn((GraphNode, NodePosition::default(), Node::default(), ChildOf(content))).id();
    let handle = w.spawn((NodeDragHandle, Node::default(), ChildOf(node))).id();
    let body = w.spawn((Node::default(), ChildOf(node))).id();

    drag(w, body, Vec2::new(10.0, 0.0));
    assert_eq!(w.get::<NodePosition>(node).unwrap().0, Vec2::ZERO);
    drag(w, handle, Vec2::new(10.0, 0.0));
    assert_eq!(w.get::<NodePosition>(node).unwrap().0, Vec2::new(10.0, 0.0));
}

#[test]
fn drags_stream_steps_and_end_with_their_total() {
    let mut app = app();
    app.add_message::<EditApplied>();
    let w = app.world_mut();
    let (_, content) = graph(w);
    let node = w.spawn((GraphNode, NodePosition::default(), Node::default(), ChildOf(content))).id();
    drag(w, node, Vec2::new(10.0, 4.0));
    let moves: Vec<_> = w
        .resource_mut::<Messages<EditApplied>>()
        .drain()
        .filter_map(|e| match e.change {
            GraphChange::Moved { drag, .. } => Some((e.change.is_drag_step(), drag)),
            _ => None,
        })
        .collect();
    let progress = |is_final| Some(DragProgress { total: Vec2::new(10.0, 4.0), is_final });
    assert_eq!(moves, [(true, progress(false)), (false, progress(true))]);
}

#[test]
fn dragging_from_a_port_snaps_to_the_hovered_port_and_connects() {
    let mut app = app();
    let w = app.world_mut();
    let (_, content) = graph(w);
    let ports = one_node_each(w, content, [Port::output(NUM), Port::input(NUM)]);
    let button = PointerButton::Primary;
    pointer(w, ports[0], DragStart { button, hit: hit() });
    assert_eq!(w.query::<&PendingWire>().iter(w).count(), 1);

    // bevy_picking keeps hovering during a drag; the wire snaps to what it hovers.
    let hovered = [(ports[1], hit())].into_iter().collect();
    w.resource_mut::<HoverMap>().insert(PointerId::Mouse, hovered);
    pointer(w, ports[0], Drag { button, distance: Vec2::X, delta: Vec2::X });
    let target = *w.query::<&WireTarget>().single(w).unwrap();
    assert_eq!(target, WireTarget(Some(ports[1])));

    pointer(w, ports[0], DragEnd { button, distance: Vec2::X });
    let edges: Vec<_> = w.query::<&EdgeTarget>().iter(w).map(|t| t.0).collect();
    assert_eq!(edges, [ports[1]]);
    assert_eq!(w.query::<&PendingWire>().iter(w).count(), 0);
}

#[test]
fn panning_works_from_an_edge() {
    let mut app = app();
    let w = app.world_mut();
    let (canvas, content) = graph(w);
    let [from, to] = one_node_each(w, content, [Port::output(NUM), Port::input(NUM)]);
    let edit = GraphEdit::Connect { from, to };
    let Ok(GraphChange::Connected { edge, .. }) = w.graph_edit(canvas, edit) else {
        panic!("connected");
    };
    // Edges have no parent, so their events never bubble to the canvas.
    let button = PointerButton::Middle;
    let by = Vec2::new(30.0, -10.0);
    pointer(w, edge, DragStart { button, hit: hit() });
    let (distance, delta) = (by, by);
    pointer(w, edge, Drag { button, distance, delta });
    let pan = w.get::<CanvasView>(canvas).unwrap().pan;
    assert!((pan - by).length() < 1e-3, "{pan}");
}

#[test]
fn pinching_zooms_the_innermost_canvas_under_the_mouse() {
    let mut app = app();
    let w = app.world_mut();
    let (outer, content) = graph(w);
    let node = w.spawn((GraphNode, Node::default(), ChildOf(content))).id();
    let inner = w.spawn((NodeCanvas, CanvasInteraction::default(), laid_out(), ChildOf(node))).id();
    let location = Location { target: NormalizedRenderTarget::None { width: 1, height: 1 }, position: Vec2::ZERO };
    w.spawn((PointerId::Mouse, PointerLocation::new(location)));
    let hovered = [(inner, hit())].into_iter().collect();
    w.resource_mut::<HoverMap>().insert(PointerId::Mouse, hovered);
    w.write_message(PinchGesture(0.5));
    app.update();
    let zoom = |app: &App, canvas| app.world().get::<CanvasView>(canvas).unwrap().zoom;
    assert_eq!((zoom(&app, inner), zoom(&app, outer)), (1.5, 1.0));

    // Not while interaction is disabled.
    app.world_mut().entity_mut(inner).insert(InteractionDisabled);
    app.world_mut().write_message(PinchGesture(0.5));
    app.update();
    assert_eq!(zoom(&app, inner), 1.5);
}

#[test]
fn dragged_wires_snap_to_ports_observers_allow() {
    let mut app = app();
    const TEXT: PortType = PortType::named("text");
    app.add_observer(|mut check: On<ConnectionCheck>| {
        if check.refused == Some(bevy_noodle::RejectReason::IncompatibleTypes) {
            check.allow();
        }
    });
    let w = app.world_mut();
    let (_, content) = graph(w);
    let ports = one_node_each(w, content, [Port::output(NUM), Port::input(TEXT)]);
    let button = PointerButton::Primary;
    pointer(w, ports[0], DragStart { button, hit: hit() });
    assert!(candidate(w, ports[1]), "the observer allows it");
    let hovered = [(ports[1], hit())].into_iter().collect();
    w.resource_mut::<HoverMap>().insert(PointerId::Mouse, hovered);
    let (distance, delta) = (Vec2::X, Vec2::X);
    pointer(w, ports[0], Drag { button, distance, delta });
    pointer(w, ports[0], DragEnd { button, distance });
    let targets: Vec<_> = w.query::<&EdgeTarget>().iter(w).map(|t| t.0).collect();
    assert_eq!(targets, [ports[1]]);
}

/// Feeds the edge picking backend a UI hit on `top` at `at`, and returns the
/// edge it reports, if any.
fn pick(app: &mut App, top: Entity, at: Vec2) -> Option<Entity> {
    use bevy::picking::backend::PointerHits;
    let w = app.world_mut();
    let camera = w.spawn(Camera::default()).id();
    let location = Location { target: NormalizedRenderTarget::None { width: 1, height: 1 }, position: at };
    w.spawn((PointerId::Mouse, PointerLocation::new(location)));
    let hit = HitData::new(camera, 0.0, None, None);
    w.write_message(PointerHits::new(PointerId::Mouse, vec![(top, hit)], 0.5));
    app.update();
    let messages = app.world().resource::<Messages<PointerHits>>();
    let mut cursor = messages.get_cursor();
    let picks: Vec<Entity> = cursor.read(messages).flat_map(|h| h.picks.iter().map(|(e, _)| *e)).collect();
    // The latest hit: the previous frame's messages are still buffered.
    picks.into_iter().rev().find(|e| app.world().get::<Edge>(*e).is_some())
}

#[test]
fn outer_edges_are_pickable_over_nested_canvases() {
    let mut app = app();
    let w = app.world_mut();
    let (_, outer) = graph(w);
    let group = w.spawn((GraphNode, Node::default(), ChildOf(outer))).id();
    let inner_canvas = w.spawn((NodeCanvas, CanvasInteraction::default(), Node::default(), ChildOf(group))).id();
    w.flush();
    let inner = w.get::<Children>(inner_canvas).unwrap()[0];
    // An edge of `content` running along y = 0, from x = 0 to x = 100.
    let edge_in = |w: &mut World, content: Entity| {
        let ports = one_node_each(w, content, [Port::output(NUM), Port::input(NUM)]);
        let points = [Vec2::ZERO, Vec2::new(30.0, 0.0), Vec2::new(70.0, 0.0), Vec2::new(100.0, 0.0)];
        let hitbox = EdgeHitbox { points, radius: 4.0, below_nodes: false };
        w.spawn((Edge, EdgeSource(ports[0]), EdgeTarget(ports[1]), hitbox)).id()
    };
    let outer_edge = edge_in(w, outer);
    // Over the nested canvas, where only the outer edge passes.
    assert_eq!(pick(&mut app, inner_canvas, Vec2::new(50.0, 1.0)), Some(outer_edge));
    // Where an inner edge passes too, the inner one wins.
    let w = app.world_mut();
    let inner_edge = edge_in(w, inner);
    assert_eq!(pick(&mut app, inner_canvas, Vec2::new(50.0, 1.0)), Some(inner_edge));
}

#[test]
fn edges_above_nodes_do_not_intercept_controls_or_their_children() {
    for target_index in 0..3 {
        let mut app = app();
        let w = app.world_mut();
        let (_, content) = graph(w);
        let ports = one_node_each(w, content, [Port::output(NUM), Port::input(NUM)]);
        let node = w.get::<ChildOf>(ports[0]).unwrap().parent();
        // Keyboard-enabled graph nodes are tabbable; that must not stop
        // edges from being picked over their ordinary backgrounds.
        w.entity_mut(node).insert(TabIndex(0));
        let control = w.spawn((TabIndex(0), Node::default(), ChildOf(node))).id();
        let child = w.spawn((Node::default(), ChildOf(control))).id();
        let edge = w
            .spawn((
                Edge,
                EdgeSource(ports[0]),
                EdgeTarget(ports[1]),
                EdgeHitbox {
                    points: [Vec2::ZERO, Vec2::new(30.0, 0.0), Vec2::new(70.0, 0.0), Vec2::new(100.0, 0.0)],
                    radius: 4.0,
                    below_nodes: false,
                },
            ))
            .id();
        let target = [node, control, child][target_index];
        let expected = (target == node).then_some(edge);
        assert_eq!(pick(&mut app, target, Vec2::new(50.0, 1.0)), expected);
    }
}

#[test]
fn controls_inside_nodes_keep_their_presses_and_drags() {
    let mut app = app();
    let w = app.world_mut();
    let (_, content) = graph(w);
    let node = w.spawn((GraphNode, NodePosition::default(), Node::default(), ChildOf(content))).id();
    // A focusable control, such as a slider or text field, and a label.
    let control = w.spawn((TabIndex(0), Node::default(), ChildOf(node))).id();
    let thumb = w.spawn((Node::default(), ChildOf(control))).id();
    let label = w.spawn((Text::new("Value"), ChildOf(node))).id();

    let button = PointerButton::Primary;
    pointer(w, thumb, Press { button, hit: hit(), count: 1 });
    drag(w, thumb, Vec2::new(10.0, 0.0));
    assert_eq!(w.get::<NodePosition>(node).unwrap().0, Vec2::ZERO);
    assert!(w.get::<Selected>(node).is_none());

    pointer(w, label, Press { button, hit: hit(), count: 1 });
    drag(w, label, Vec2::new(10.0, 0.0));
    assert_eq!(w.get::<NodePosition>(node).unwrap().0, Vec2::new(10.0, 0.0));
    assert!(w.get::<Selected>(node).is_some());
}

#[test]
fn wires_mark_candidates_of_their_canvas_while_they_exist() {
    let mut app = app();
    let w = app.world_mut();
    let [(first, fits, other), (_, second_fits, _)] = [(); 2].map(|()| {
        let (canvas, content) = graph(w);
        let ports = [Port::output(NUM), Port::input(NUM), Port::output(NUM)];
        let [from, fits, other] = one_node_each(w, content, ports);
        let pointer = Vec2::ZERO;
        let wire = w.spawn((PendingWire { from, pointer }, WireOf(canvas)));
        (wire.id(), fits, other)
    });
    w.flush();
    assert!(candidate(w, fits));
    assert!(!candidate(w, other));
    w.despawn(first);
    w.flush();
    assert!(!candidate(w, fits));
    assert!(candidate(w, second_fits), "another canvas keeps its marks");
}

#[test]
fn dragging_off_a_connected_input_picks_up_its_wire() {
    let mut app = app();
    let w = app.world_mut();
    let (canvas, content) = graph(w);
    let [output, input] = one_node_each(w, content, [Port::output(NUM), Port::input(NUM)]);
    w.graph_edit(canvas, GraphEdit::Connect { from: output, to: input }).unwrap();
    let button = PointerButton::Primary;
    pointer(w, input, DragStart { button, hit: hit() });
    let wire = *w.query::<&PendingWire>().single(w).unwrap();
    assert_eq!(wire.from, output, "the wire hangs off the output now");
    assert_eq!(w.query::<&Edge>().iter(w).count(), 0);
    assert!(candidate(w, input));

    let distance = Vec2::X;
    pointer(w, input, DragEnd { button, distance });
    assert_eq!(w.query::<&PendingWire>().iter(w).count(), 0);
    assert_eq!(w.query::<&Edge>().iter(w).count(), 0, "dropped on nothing");
    assert!(!candidate(w, input));
}

#[test]
fn pressing_raises_nodes_without_reordering_them() {
    let mut app = app();
    let w = app.world_mut();
    let (_, content) = graph(w);
    let spawn = |w: &mut World, z: i32| {
        let node = (GraphNode, Node::default(), ZIndex(z), ChildOf(content));
        w.spawn(node).id()
    };
    let (a, b, frame) = (spawn(w, 0), spawn(w, 0), spawn(w, -1));
    let order = w.get::<Children>(content).unwrap().to_vec();
    let press = |w: &mut World, node| {
        let button = PointerButton::Primary;
        let (hit, count) = (hit(), 1);
        pointer(w, node, Press { button, hit, count });
    };
    let z = |w: &World, node| w.get::<ZIndex>(node).unwrap().0;
    press(w, a);
    assert!(z(w, a) > z(w, b));
    press(w, b);
    assert!(z(w, b) > z(w, a));
    press(w, b);
    assert_eq!(z(w, b), 2, "already on top");
    press(w, frame);
    assert_eq!(z(w, frame), -1, "negative ZIndex stays under");
    assert_eq!(w.get::<Children>(content).unwrap().to_vec(), order);
}

#[test]
fn wires_dropped_on_empty_canvas_are_reported() {
    let mut app = app();
    #[derive(Resource, Default)]
    struct Dropped(Vec<Entity>);
    app.init_resource::<Dropped>();
    app.add_observer(|d: On<WireDropped>, mut dropped: ResMut<Dropped>| dropped.0.push(d.from));
    let w = app.world_mut();
    let (canvas, content) = graph(w);
    let [out] = one_node_each(w, content, [Port::output(NUM)]);
    drag(w, out, Vec2::new(40.0, 0.0));
    assert_eq!(w.resource::<Dropped>().0, [out]);
    let messages: Vec<_> = w.resource_mut::<Messages<WireDropped>>().drain().collect();
    assert_eq!((messages.len(), messages[0].canvas), (1, canvas));
}

#[test]
fn drags_follow_the_pointer_through_an_outer_zoom() {
    let mut app = app();
    let w = app.world_mut();
    let (canvas, content) = graph(w);
    // As inside an outer canvas zoomed in twice.
    w.entity_mut(canvas).insert(UiGlobalTransform::from_scale(Vec2::splat(2.0)));
    let node = w.spawn((GraphNode, NodePosition::default(), Node::default(), ChildOf(content))).id();
    drag(w, node, Vec2::new(10.0, 0.0));
    assert_eq!(w.get::<NodePosition>(node).unwrap().0, Vec2::new(5.0, 0.0));
}

#[test]
fn wires_follow_the_pointer_under_ui_scale() {
    let mut app = app();
    let w = app.world_mut();
    w.insert_resource(UiScale(2.0));
    let (canvas, content) = graph(w);
    // Layout under `UiScale` 2: physical sizes, the top left at the origin.
    let size = Vec2::new(400.0, 300.0);
    let computed = ComputedNode { size, inverse_scale_factor: 0.5, ..default() };
    let at_origin = UiGlobalTransform::from_translation(size / 2.0);
    w.entity_mut(canvas).insert((computed, at_origin));
    let [port] = one_node_each(w, content, [Port::output(NUM)]);
    let button = PointerButton::Primary;
    pointer_at(w, port, Vec2::new(100.0, 50.0), DragStart { button, hit: hit() });
    let wire = w.query::<&PendingWire>().single(w).unwrap();
    assert!((wire.pointer - Vec2::new(50.0, 25.0)).length() < 1e-3);
}

#[test]
fn cancelled_pointers_drop_their_wire() {
    let mut app = app();
    let w = app.world_mut();
    let (_, content) = graph(w);
    let [port] = one_node_each(w, content, [Port::output(NUM)]);
    let button = PointerButton::Primary;
    pointer(w, port, DragStart { button, hit: hit() });
    assert_eq!(w.query::<&PendingWire>().iter(w).count(), 1);
    pointer(w, port, Cancel { hit: hit() });
    assert_eq!(w.query::<&PendingWire>().iter(w).count(), 0);
}

#[test]
fn wires_go_with_their_port() {
    let mut app = app();
    let w = app.world_mut();
    let (_, content) = graph(w);
    let [port] = one_node_each(w, content, [Port::output(NUM)]);
    let button = PointerButton::Primary;
    pointer(w, port, DragStart { button, hit: hit() });
    let node = w.get::<ChildOf>(port).unwrap().parent();
    w.despawn(node);
    app.update();
    let w = app.world_mut();
    assert_eq!(w.query::<&PendingWire>().iter(w).count(), 0);
}

#[test]
fn additive_box_selection_shrinks_back_to_what_was_selected() {
    let mut app = app();
    let w = app.world_mut();
    let (canvas, content) = graph(w);
    let size = ComputedNode { size: Vec2::splat(10.0), ..default() };
    let [a, b] = [0.0, 100.0].map(|x| {
        let at = NodePosition(Vec2::new(x, 0.0));
        w.spawn((GraphNode, at, Node::default(), size, ChildOf(content))).id()
    });
    w.select(canvas, vec![a], SelectMode::Replace);
    w.resource_mut::<ButtonInput<KeyCode>>().press(KeyCode::ShiftLeft);
    // The canvas is centred on the window's origin: window = local - (200, 150).
    let window = |local: Vec2| local - Vec2::new(200.0, 150.0);
    let start = Vec2::new(-10.0, -10.0);
    let button = PointerButton::Primary;
    pointer_at(w, canvas, window(start), DragStart { button, hit: hit() });
    let selected = |w: &mut World, to: Vec2| {
        let (distance, delta) = (to - start, to - start);
        pointer_at(w, canvas, window(to), Drag { button, distance, delta });
        let mut selected = w.query_filtered::<Entity, With<Selected>>();
        selected.iter(w).collect::<EntityHashSet>()
    };
    assert_eq!(selected(w, Vec2::new(120.0, 20.0)), EntityHashSet::from_iter([a, b]));
    assert_eq!(selected(w, Vec2::new(50.0, 20.0)), EntityHashSet::from_iter([a]));
}

// Moving edges: with a `move_wires_keys` key held, a drag from a connected
// port lifts its edges instead of dragging a new wire.

/// A canvas moving edges while Ctrl is held, Ctrl held, and one node per
/// port, laid out along x: port `i` at `(100 i, 0)` in graph space.
fn moving<const N: usize>(app: &mut App, ports: [Port; N]) -> (Entity, [Entity; N]) {
    let w = app.world_mut();
    let (canvas, content) = graph(w);
    let settings = CanvasInteraction { move_wires_keys: vec![KeyCode::ControlLeft], ..default() };
    w.entity_mut(canvas).insert(settings);
    let ports = one_node_each(w, content, ports);
    // Headless, layout measures nothing: place the ports by hand afterwards.
    app.update();
    let w = app.world_mut();
    for (i, port) in ports.iter().enumerate() {
        let at = Vec2::new(100.0 * i as f32, 0.0);
        w.entity_mut(*port).insert(PortAnchor::new(None, at, Some(at)));
    }
    w.resource_mut::<ButtonInput<KeyCode>>().press(KeyCode::ControlLeft);
    (canvas, ports)
}

fn edge(w: &mut World, canvas: Entity, from: Entity, to: Entity) -> Entity {
    match w.graph_edit(canvas, GraphEdit::Connect { from, to }) {
        Ok(GraphChange::Connected { edge, .. }) => edge,
        other => panic!("not connected: {other:?}"),
    }
}

/// The canvas is centred on the window's origin: graph point → window position.
fn window(graph: Vec2) -> Vec2 {
    graph - Vec2::new(200.0, 150.0)
}

/// What bevy_picking hovers: `port`, or nothing.
fn hover(w: &mut World, port: Option<Entity>) {
    let hovered = port.map(|p| (p, hit())).into_iter().collect();
    w.resource_mut::<HoverMap>().insert(PointerId::Mouse, hovered);
}

/// A drag step from `port` with the pointer at graph point `to`.
fn drag_to(w: &mut World, port: Entity, to: Vec2) {
    let (button, distance, delta) = (PointerButton::Primary, Vec2::X, Vec2::X);
    pointer_at(w, port, window(to), Drag { button, distance, delta });
}

fn lift(w: &mut World, port: Entity) {
    pointer(w, port, DragStart { button: PointerButton::Primary, hit: hit() });
}

fn moving_edges(w: &mut World) -> Vec<MovingEdges> {
    w.query::<&MovingEdges>().iter(w).cloned().collect()
}

fn applied(w: &mut World) -> Vec<GraphChange> {
    w.resource_mut::<Messages<EditApplied>>().drain().map(|e| e.change).collect()
}

fn geometry(app: &mut App, edge: Entity) -> EdgeGeometry {
    app.update();
    *app.world().get::<EdgeGeometry>(edge).unwrap()
}

#[test]
fn lifting_an_input_takes_its_edge_without_an_edit() {
    let mut app = app();
    let (canvas, [out, inp]) = moving(&mut app, [Port::output(NUM), Port::input(NUM)]);
    let w = app.world_mut();
    let edge = edge(w, canvas, out, inp);
    applied(w);
    lift(w, inp);
    let lifted = moving_edges(w);
    assert_eq!(lifted.len(), 1);
    assert_eq!((lifted[0].from, lifted[0].edges.clone()), (inp, vec![edge]));
    assert_eq!(w.query::<&PendingWire>().iter(w).count(), 0, "no new wire");
    assert_eq!(w.get::<EdgeTarget>(edge).unwrap().0, inp, "still connected");
    assert_eq!(applied(w), [], "no edit");
}

#[test]
fn lifting_an_output_takes_all_its_edges() {
    let mut app = app();
    let ports = [Port::output(NUM), Port::input(NUM), Port::input(NUM)];
    let (canvas, [out, a, b]) = moving(&mut app, ports);
    let w = app.world_mut();
    let edges = vec![edge(w, canvas, out, a), edge(w, canvas, out, b)];
    lift(w, out);
    let lifted = moving_edges(w);
    assert_eq!((lifted[0].from, lifted[0].edges.clone()), (out, edges));
    assert_eq!(w.query::<&PendingWire>().iter(w).count(), 0, "no new wire");
}

#[test]
fn lifted_edges_follow_the_pointer() {
    let mut app = app();
    let ports = [Port::output(NUM), Port::input(NUM), Port::input(NUM)];
    let (canvas, [out, a, b]) = moving(&mut app, ports);
    let w = app.world_mut();
    let into_a = edge(w, canvas, out, a);
    let into_b = edge(w, canvas, out, b);
    lift(w, a);
    let at = Vec2::new(50.0, 80.0);
    drag_to(app.world_mut(), a, at);
    let lifted = geometry(&mut app, into_a);
    assert_eq!((lifted.start, lifted.output), (Vec2::ZERO, Some(out)), "the far end stays");
    assert!((lifted.end - at).length() < 1e-3, "{}", lifted.end);
    assert_eq!((lifted.end_tangent, lifted.input), (Vec2::NEG_X, None), "enters like an input, at no port");
    assert_eq!(geometry(&mut app, into_b).end, Vec2::new(200.0, 0.0), "other edges stay");

    // An output's edges leave the pointer like an output.
    let w = app.world_mut();
    pointer(w, a, DragEnd { button: PointerButton::Primary, distance: Vec2::X });
    lift(w, out);
    drag_to(w, out, at);
    for edge in [into_a, into_b] {
        let lifted = geometry(&mut app, edge);
        assert!((lifted.start - at).length() < 1e-3, "{}", lifted.start);
        assert_eq!((lifted.start_tangent, lifted.output), (Vec2::X, None));
    }
}

#[test]
fn lifted_edges_snap_only_to_accepted_candidates() {
    let mut app = app();
    let ports = [Port::output(NUM), Port::input(NUM), Port::input(NUM), Port::input(NUM), Port::output(NUM)];
    let (canvas, [out, from, accepted, refused, other_way]) = moving(&mut app, ports);
    app.world_mut().add_observer(move |mut check: On<MoveCheck>| {
        if check.to == refused {
            check.reject();
        }
    });
    let w = app.world_mut();
    let edge = edge(w, canvas, out, from);
    lift(w, from);
    assert!(candidate(w, accepted));
    assert!(!candidate(w, refused), "the observer refuses it");
    assert!(!candidate(w, other_way) && !candidate(w, from) && !candidate(w, out));

    let at = Vec2::new(50.0, 80.0);
    hover(w, Some(refused));
    drag_to(w, from, at);
    assert_eq!(*w.query::<&WireTarget>().single(w).unwrap(), WireTarget(None));
    assert!((geometry(&mut app, edge).end - at).length() < 1e-3, "no snap");

    let w = app.world_mut();
    hover(w, Some(accepted));
    drag_to(w, from, at);
    assert_eq!(*w.query::<&WireTarget>().single(w).unwrap(), WireTarget(Some(accepted)));
    let snapped = geometry(&mut app, edge);
    assert_eq!((snapped.end, snapped.end_tangent, snapped.input), (Vec2::new(200.0, 0.0), Vec2::NEG_X, Some(accepted)));
}

#[test]
fn dropping_lifted_edges_on_a_candidate_requests_one_move() {
    let mut app = app();
    #[derive(Resource, Default)]
    struct Requests(Vec<GraphEdit>);
    app.init_resource::<Requests>();
    app.add_observer(|r: On<EditRequested>, mut requests: ResMut<Requests>| requests.0.push(r.edit.clone()));
    let ports = [Port::output(NUM), Port::input(NUM), Port::input(NUM)];
    let (canvas, [out, from, to]) = moving(&mut app, ports);
    let w = app.world_mut();
    let edge = edge(w, canvas, out, from);
    w.resource_mut::<Requests>().0.clear();
    applied(w);
    lift(w, from);
    hover(w, Some(to));
    drag_to(w, from, Vec2::new(200.0, 0.0));
    pointer(w, from, DragEnd { button: PointerButton::Primary, distance: Vec2::X });
    let requests = std::mem::take(&mut w.resource_mut::<Requests>().0);
    assert_eq!(requests, [GraphEdit::MoveEdges { from, to, edges: vec![edge] }]);
    assert_eq!(applied(w), [GraphChange::EdgesMoved { edges: vec![edge], from, to }]);
    assert_eq!(w.get::<EdgeTarget>(edge).unwrap().0, to, "the same edge, moved");
    assert_eq!(moving_edges(w), []);
}

#[test]
fn lifted_edges_dropped_elsewhere_or_cancelled_go_back() {
    for cancel in [false, true] {
        let mut app = app();
        #[derive(Resource, Default)]
        struct Dropped(Vec<EdgesDropped>);
        app.init_resource::<Dropped>();
        app.add_observer(|d: On<EdgesDropped>, mut dropped: ResMut<Dropped>| dropped.0.push((*d).clone()));
        let ports = [Port::output(NUM), Port::input(NUM), Port::input(NUM)];
        let (canvas, [out, from, _]) = moving(&mut app, ports);
        let w = app.world_mut();
        let edge = edge(w, canvas, out, from);
        applied(w);
        lift(w, from);
        let at = Vec2::new(50.0, 80.0);
        drag_to(w, from, at);
        assert!((geometry(&mut app, edge).end - at).length() < 1e-3);

        let w = app.world_mut();
        match cancel {
            true => pointer(w, from, Cancel { hit: hit() }),
            false => pointer_at(w, from, window(at), DragEnd { button: PointerButton::Primary, distance: Vec2::X }),
        }
        assert_eq!(moving_edges(w), []);
        assert_eq!(applied(w), [], "no edit");
        assert_eq!(w.get::<EdgeTarget>(edge).unwrap().0, from);
        let dropped = std::mem::take(&mut w.resource_mut::<Dropped>().0);
        match cancel {
            true => assert!(dropped.is_empty()),
            false => {
                assert_eq!(dropped.len(), 1);
                let d = &dropped[0];
                assert_eq!((d.canvas, d.from, d.edges.clone()), (canvas, from, vec![edge]));
                assert!((d.position - at).length() < 1e-3);
            }
        }
        let back = geometry(&mut app, edge);
        assert_eq!((back.end, back.input), (Vec2::new(100.0, 0.0), Some(from)), "back at its port");
    }
}

#[test]
fn move_keys_are_unbound_by_default() {
    let mut app = app();
    let (canvas, [out, inp]) = moving(&mut app, [Port::output(NUM), Port::input(NUM)]);
    let w = app.world_mut();
    w.entity_mut(canvas).insert(CanvasInteraction::default());
    edge(w, canvas, out, inp);
    // Ctrl is held, but no key moves edges: the input's wire is picked up as upstream does.
    lift(w, inp);
    assert_eq!(moving_edges(w), []);
    assert_eq!(w.query::<&PendingWire>().single(w).unwrap().from, out);
    assert_eq!(w.query::<&Edge>().iter(w).count(), 0);
}

#[test]
fn move_keys_on_an_unconnected_port_drag_a_new_wire() {
    let mut app = app();
    let (_, [out, _]) = moving(&mut app, [Port::output(NUM), Port::input(NUM)]);
    let w = app.world_mut();
    lift(w, out);
    assert_eq!(moving_edges(w), []);
    assert_eq!(w.query::<&PendingWire>().single(w).unwrap().from, out);
}

// Node drags: snapping to a grid, and frames carrying the nodes inside them.

/// A node of `size` at `at`, laid out (headless, layout measures nothing).
fn sized_node(w: &mut World, content: Entity, at: Vec2, size: Vec2) -> Entity {
    let computed = ComputedNode { size, inverse_scale_factor: 1.0, ..default() };
    w.spawn((GraphNode, NodePosition(at), Node::default(), computed, ChildOf(content))).id()
}

fn position(w: &World, node: Entity) -> Vec2 {
    w.get::<NodePosition>(node).unwrap().0
}

/// A drag step of `node`: the pointer moved `by` since the last step, `total` since the drag started.
fn drag_step(w: &mut World, node: Entity, by: Vec2, total: Vec2) {
    pointer(w, node, Drag { button: PointerButton::Primary, distance: total, delta: by });
}

fn close(a: Vec2, b: Vec2) -> bool {
    (a - b).length() < 1e-4
}

#[test]
fn snap_is_off_by_default() {
    assert_eq!(CanvasInteraction::default().snap, None);
}

#[test]
fn snapped_drags_put_the_grabbed_node_on_the_grid() {
    let mut app = app();
    let w = app.world_mut();
    let (canvas, content) = graph(w);
    w.get_mut::<CanvasInteraction>(canvas).unwrap().snap = Some(10.0);
    let (a, b) = (
        sized_node(w, content, Vec2::new(12.3, 7.6), Vec2::ONE),
        sized_node(w, content, Vec2::new(103.0, 51.0), Vec2::ONE),
    );
    w.select(canvas, vec![a, b], SelectMode::Replace);
    let rel = position(w, b) - position(w, a);
    let button = PointerButton::Primary;
    pointer(w, a, DragStart { button, hit: hit() });
    drag_step(w, a, Vec2::new(1.0, 0.5), Vec2::new(1.0, 0.5));
    assert_eq!(position(w, a), Vec2::new(10.0, 10.0), "on the grid from the first step");
    drag_step(w, a, Vec2::new(1.0, 0.5), Vec2::new(2.0, 1.0));
    assert_eq!(position(w, a), Vec2::new(10.0, 10.0), "until another grid point is nearer");
    drag_step(w, a, Vec2::new(20.0, -30.0), Vec2::new(22.0, -29.0));
    assert_eq!(position(w, a), Vec2::new(30.0, -20.0));
    pointer(w, a, DragEnd { button, distance: Vec2::new(22.0, -29.0) });
    assert_eq!(position(w, a), Vec2::new(30.0, -20.0));
    assert!(close(position(w, b) - position(w, a), rel), "the selection keeps its layout");

    // The grabbed node leads, whichever node is selected first: b (120.7, 23.4) + (4, 0) → (120, 20).
    pointer(w, b, DragStart { button, hit: hit() });
    drag_step(w, b, Vec2::new(4.0, 0.0), Vec2::new(4.0, 0.0));
    pointer(w, b, DragEnd { button, distance: Vec2::new(4.0, 0.0) });
    assert_eq!(position(w, b), Vec2::new(120.0, 20.0));
}

#[test]
fn each_drag_snaps_from_where_the_node_is_when_it_starts() {
    let mut app = app();
    let w = app.world_mut();
    let (canvas, content) = graph(w);
    w.get_mut::<CanvasInteraction>(canvas).unwrap().snap = Some(10.0);
    let node = sized_node(w, content, Vec2::new(12.3, 7.6), Vec2::ONE);
    let button = PointerButton::Primary;
    // A drag that never ends (a touch taken over without a cancel).
    pointer(w, node, DragStart { button, hit: hit() });
    drag_step(w, node, Vec2::new(1.0, 0.5), Vec2::new(1.0, 0.5));
    assert_eq!(position(w, node), Vec2::new(10.0, 10.0));
    // The next one starts from (10, 10), not from (12.3, 7.6): 10 + 14 → 20, not 12.3 + 14 → 30.
    pointer(w, node, DragStart { button, hit: hit() });
    drag_step(w, node, Vec2::new(14.0, 0.0), Vec2::new(14.0, 0.0));
    assert_eq!(position(w, node), Vec2::new(20.0, 10.0));
    // A cancelled one leaves nothing behind either.
    pointer(w, node, Cancel { hit: hit() });
    pointer(w, node, DragStart { button, hit: hit() });
    drag_step(w, node, Vec2::new(0.0, 6.0), Vec2::new(0.0, 6.0));
    assert_eq!(position(w, node), Vec2::new(20.0, 20.0));
}

/// A frame at the origin, 200 by 150, with a node inside it, one sticking out of it and one beside it.
fn framed(w: &mut World, content: Entity) -> [Entity; 4] {
    let frame = sized_node(w, content, Vec2::ZERO, Vec2::new(200.0, 150.0));
    w.entity_mut(frame).insert(NodeFrame);
    let inside = sized_node(w, content, Vec2::new(20.0, 30.0), Vec2::new(50.0, 40.0));
    let across = sized_node(w, content, Vec2::new(180.0, 0.0), Vec2::new(50.0, 40.0));
    let beside = sized_node(w, content, Vec2::new(230.0, 30.0), Vec2::new(50.0, 40.0));
    [frame, inside, across, beside]
}

#[test]
fn dragging_a_frame_carries_the_nodes_inside_it() {
    let mut app = app();
    app.add_message::<EditApplied>();
    let w = app.world_mut();
    let (_, content) = graph(w);
    let [frame, inside, across, beside] = framed(w, content);
    drag(w, frame, Vec2::new(15.0, 5.0));
    assert_eq!(position(w, frame), Vec2::new(15.0, 5.0));
    assert_eq!(position(w, inside), Vec2::new(35.0, 35.0), "carried");
    assert_eq!(position(w, across), Vec2::new(180.0, 0.0), "only nodes entirely inside");
    assert_eq!(position(w, beside), Vec2::new(230.0, 30.0));
    // Each step is one edit moving the frame and what it carries.
    for change in applied(w) {
        let GraphChange::Moved { nodes, .. } = change else { panic!("{change:?}") };
        assert_eq!(EntityHashSet::from_iter(nodes), EntityHashSet::from_iter([frame, inside]));
    }

    // A node inside does not drag its frame.
    drag(w, inside, Vec2::new(5.0, 0.0));
    assert_eq!((position(w, inside), position(w, frame)), (Vec2::new(40.0, 35.0), Vec2::new(15.0, 5.0)));
}

#[test]
fn frames_carry_what_was_inside_when_the_drag_started() {
    let mut app = app();
    let w = app.world_mut();
    let (_, content) = graph(w);
    let [frame, inside, _, beside] = framed(w, content);
    let button = PointerButton::Primary;
    pointer(w, frame, DragStart { button, hit: hit() });
    // Over `beside` (frame at 100..300), then on: `beside` stays where it was.
    drag_step(w, frame, Vec2::new(100.0, 0.0), Vec2::new(100.0, 0.0));
    drag_step(w, frame, Vec2::new(50.0, 0.0), Vec2::new(150.0, 0.0));
    pointer(w, frame, DragEnd { button, distance: Vec2::new(150.0, 0.0) });
    assert_eq!(position(w, beside), Vec2::new(230.0, 30.0));
    assert_eq!(position(w, inside), Vec2::new(170.0, 30.0));
}

#[test]
fn a_snapped_frame_moves_its_contents_by_its_own_delta() {
    let mut app = app();
    let w = app.world_mut();
    let (canvas, content) = graph(w);
    w.get_mut::<CanvasInteraction>(canvas).unwrap().snap = Some(10.0);
    let frame = sized_node(w, content, Vec2::new(2.0, 3.0), Vec2::new(400.0, 300.0));
    w.entity_mut(frame).insert(NodeFrame);
    let node = sized_node(w, content, Vec2::new(12.3, 7.6), Vec2::new(100.0, 50.0));
    pointer(w, frame, DragStart { button: PointerButton::Primary, hit: hit() });
    drag_step(w, frame, Vec2::new(4.0, 2.5), Vec2::new(4.0, 2.5));
    assert_eq!(position(w, frame), Vec2::new(10.0, 10.0), "the frame's origin snaps");
    assert!(close(position(w, node), Vec2::new(20.3, 14.6)), "not snapped itself: {}", position(w, node));
    drag_step(w, frame, Vec2::new(12.0, 5.5), Vec2::new(16.0, 8.0));
    assert_eq!(position(w, frame), Vec2::new(20.0, 10.0));
    assert!(close(position(w, node), Vec2::new(30.3, 14.6)), "{}", position(w, node));
}

// Drops away from any candidate name the port under the pointer, if any.

#[test]
fn wires_dropped_on_a_refused_port_name_it() {
    let mut app = app();
    let w = app.world_mut();
    let (_, content) = graph(w);
    let [out, other] = one_node_each(w, content, [Port::output(NUM), Port::output(NUM)]);
    let button = PointerButton::Primary;
    let drop_over = |w: &mut World, port: Option<Entity>| {
        pointer(w, out, DragStart { button, hit: hit() });
        hover(w, port);
        pointer(w, out, Drag { button, distance: Vec2::X, delta: Vec2::X });
        pointer(w, out, DragEnd { button, distance: Vec2::X });
        let dropped: Vec<_> = w.resource_mut::<Messages<WireDropped>>().drain().collect();
        assert_eq!(dropped.len(), 1, "two outputs never connect: a drop, not a connection");
        dropped[0].onto
    };
    assert_eq!(drop_over(w, Some(other)), Some(other));
    assert_eq!(drop_over(w, None), None, "empty canvas");
    assert_eq!(drop_over(w, Some(out)), None, "its own port");
}

#[test]
fn lifted_edges_dropped_on_a_refused_port_name_it() {
    let mut app = app();
    let ports = [Port::output(NUM), Port::input(NUM), Port::output(NUM)];
    let (canvas, [out, from, refused]) = moving(&mut app, ports);
    let w = app.world_mut();
    edge(w, canvas, out, from);
    lift(w, from);
    hover(w, Some(refused));
    drag_to(w, from, Vec2::new(200.0, 0.0));
    pointer_at(w, from, window(Vec2::new(200.0, 0.0)), DragEnd { button: PointerButton::Primary, distance: Vec2::X });
    let dropped: Vec<_> = w.resource_mut::<Messages<EdgesDropped>>().drain().collect();
    assert_eq!(dropped.len(), 1);
    assert_eq!(dropped[0].onto, Some(refused), "an output, where an input's edges cannot go");
}

// The nearest candidate within a radius: with `wire_target_radius`, a dragged (or lifted) wire's target is the
// candidate port whose centre is nearest to the pointer within that many window pixels, at any zoom.

/// [`moving`] on a canvas whose wires take the nearest candidate within 40 px, zoomed by `zoom` (pan zero).
fn near<const N: usize>(app: &mut App, zoom: f32, ports: [Port; N]) -> (Entity, [Entity; N]) {
    let (canvas, ports) = moving(app, ports);
    let w = app.world_mut();
    w.get_mut::<CanvasInteraction>(canvas).unwrap().wire_target_radius = Some(40.0);
    w.get_mut::<CanvasView>(canvas).unwrap().zoom = zoom;
    (canvas, ports)
}

/// Puts `port`'s centre at graph point `at`.
fn place(w: &mut World, port: Entity, at: Vec2) {
    w.entity_mut(port).insert(PortAnchor::new(None, at, Some(at)));
}

/// A drag step from `port` with the pointer at window position `at`.
fn drag_at(w: &mut World, port: Entity, at: Vec2) {
    let (button, distance, delta) = (PointerButton::Primary, Vec2::X, Vec2::X);
    pointer_at(w, port, at, Drag { button, distance, delta });
}

/// The window position `px` window pixels below graph point `at`, under `zoom` (the canvas is centred on the window's
/// origin, pan zero).
fn below(at: Vec2, zoom: f32, px: f32) -> Vec2 {
    off(at, zoom, Vec2::new(0.0, px))
}

/// The window position `by` window pixels away from graph point `at`, under `zoom`.
fn off(at: Vec2, zoom: f32, by: Vec2) -> Vec2 {
    at * zoom + by - Vec2::new(200.0, 150.0)
}

fn target(w: &mut World) -> Option<Entity> {
    w.query::<&WireTarget>().single(w).unwrap().0
}

fn end_drag(w: &mut World, port: Entity, at: Vec2) {
    pointer_at(w, port, at, DragEnd { button: PointerButton::Primary, distance: Vec2::X });
}

#[test]
fn wire_target_radius_is_off_by_default() {
    assert_eq!(CanvasInteraction::default().wire_target_radius, None);
}

#[test]
fn wires_take_a_candidate_within_the_radius_in_screen_pixels_at_any_zoom() {
    for zoom in [1.0, 0.5, 2.0] {
        let mut app = app();
        let (_, [out, inp]) = near(&mut app, zoom, [Port::output(NUM), Port::input(NUM)]);
        let w = app.world_mut();
        lift(w, out);
        // Nothing under the pointer: the distance alone decides.
        hover(w, None);
        // 35 px, diagonally (21, 28).
        let at = off(Vec2::new(100.0, 0.0), zoom, Vec2::new(-21.0, 28.0));
        drag_at(w, out, at);
        assert_eq!(target(w), Some(inp), "35 px from the input at zoom {zoom}");
        end_drag(w, out, at);
        let edges: Vec<_> = w.query::<(&EdgeSource, &EdgeTarget)>().iter(w).map(|(s, t)| (s.0, t.0)).collect();
        assert_eq!(edges, [(out, inp)], "the drop connects there at zoom {zoom}");
        assert_eq!(w.resource_mut::<Messages<WireDropped>>().drain().count(), 0);
    }
}

#[test]
fn wires_beyond_the_radius_do_not_snap() {
    for zoom in [1.0, 0.5, 2.0] {
        let mut app = app();
        let (_, [out, _]) = near(&mut app, zoom, [Port::output(NUM), Port::input(NUM)]);
        let w = app.world_mut();
        lift(w, out);
        hover(w, None);
        // 45 px, diagonally (27, -36).
        let at = off(Vec2::new(100.0, 0.0), zoom, Vec2::new(27.0, -36.0));
        drag_at(w, out, at);
        assert_eq!(target(w), None, "45 px at zoom {zoom}");
        end_drag(w, out, at);
        assert_eq!(w.query::<&Edge>().iter(w).count(), 0);
        let dropped: Vec<_> = w.resource_mut::<Messages<WireDropped>>().drain().collect();
        assert_eq!((dropped.len(), dropped[0].onto), (1, None), "an off-target drop");
    }
}

#[test]
fn the_nearest_candidate_wins() {
    let mut app = app();
    let (_, [out, far, close]) = near(&mut app, 1.0, [Port::output(NUM), Port::input(NUM), Port::input(NUM)]);
    let w = app.world_mut();
    place(w, far, Vec2::new(100.0, 35.0));
    place(w, close, Vec2::new(100.0, 0.0));
    lift(w, out);
    // Picking hovers the farther one (say, its larger dot): the distance still decides.
    hover(w, Some(far));
    drag_at(w, out, below(Vec2::new(100.0, 0.0), 1.0, 10.0));
    assert_eq!(target(w), Some(close), "10 px beats 25 px");
}

/// A refused port at 5 px and a candidate at 30 px, dropped there:
/// `refused_hovered` decides whether picking hovers the refused port.
fn refused_and_accepted(refused_hovered: bool) -> (Option<Entity>, [Entity; 2], Vec<WireDropped>) {
    let mut app = app();
    let ports = [Port::output(NUM), Port::input(NUM), Port::input(NUM)];
    let (_, [out, refused, accepted]) = near(&mut app, 1.0, ports);
    app.world_mut().add_observer(move |mut check: On<ConnectionCheck>| {
        if check.ports.input == refused {
            check.reject();
        }
    });
    let w = app.world_mut();
    place(w, refused, Vec2::new(100.0, 0.0));
    place(w, accepted, Vec2::new(100.0, 35.0));
    lift(w, out);
    assert!(!candidate(w, refused) && candidate(w, accepted));
    hover(w, refused_hovered.then_some(refused));
    let at = below(Vec2::new(100.0, 0.0), 1.0, 5.0);
    drag_at(w, out, at);
    let snapped = target(w);
    end_drag(w, out, at);
    let dropped = w.resource_mut::<Messages<WireDropped>>().drain().collect();
    (snapped, [refused, accepted], dropped)
}

#[test]
fn a_refused_port_under_the_pointer_blocks_the_snap() {
    let (target, [refused, _], dropped) = refused_and_accepted(true);
    assert_eq!(target, None, "on the refused port's box, the candidate 30 px away does not snap");
    assert_eq!(dropped.len(), 1);
    assert_eq!(dropped[0].onto, Some(refused), "the drop names it");
}

#[test]
fn a_refused_port_near_the_pointer_never_takes_the_target() {
    let (target, [_, accepted], dropped) = refused_and_accepted(false);
    assert_eq!(target, Some(accepted), "off the refused port's box, the candidate within 40 px wins");
    assert_eq!(dropped.len(), 0, "and the drop connects");
}

#[test]
fn lifted_edges_take_a_candidate_within_the_radius() {
    let mut app = app();
    #[derive(Resource, Default)]
    struct Requests(Vec<GraphEdit>);
    app.init_resource::<Requests>();
    app.add_observer(|r: On<EditRequested>, mut requests: ResMut<Requests>| requests.0.push(r.edit.clone()));
    let ports = [Port::output(NUM), Port::input(NUM), Port::input(NUM)];
    let (canvas, [out, from, to]) = near(&mut app, 2.0, ports);
    let w = app.world_mut();
    let edge = edge(w, canvas, out, from);
    w.resource_mut::<Requests>().0.clear();
    lift(w, from);
    assert_eq!(moving_edges(w).len(), 1);
    hover(w, None);
    let at = off(Vec2::new(200.0, 0.0), 2.0, Vec2::new(28.0, -21.0));
    drag_at(w, from, at);
    assert_eq!(target(w), Some(to));
    let snapped = geometry(&mut app, edge);
    assert_eq!((snapped.end, snapped.input), (Vec2::new(200.0, 0.0), Some(to)), "the loose end snaps to it");
    let w = app.world_mut();
    end_drag(w, from, at);
    let requests = std::mem::take(&mut w.resource_mut::<Requests>().0);
    assert_eq!(requests, [GraphEdit::MoveEdges { from, to, edges: vec![edge] }]);
}

#[test]
fn without_a_radius_only_the_hovered_port_is_the_target() {
    let mut app = app();
    let (canvas, [out, inp]) = moving(&mut app, [Port::output(NUM), Port::input(NUM)]);
    let w = app.world_mut();
    assert_eq!(w.get::<CanvasInteraction>(canvas).unwrap().wire_target_radius, None);
    lift(w, out);
    hover(w, None);
    drag_at(w, out, below(Vec2::new(100.0, 0.0), 1.0, 5.0));
    assert_eq!(target(w), None, "5 px off the port, not over it");
    hover(w, Some(inp));
    drag_at(w, out, below(Vec2::new(100.0, 0.0), 1.0, 5.0));
    assert_eq!(target(w), Some(inp));
}
