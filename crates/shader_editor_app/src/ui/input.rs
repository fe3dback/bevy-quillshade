//! Shortcuts (spec §9, UE-like), canvas clicks, wire drops and the minimap.

use bevy::input_focus::InputFocus;
use bevy::picking::hover::HoverMap;
use bevy::prelude::*;
use bevy::text::EditableText;
use bevy::ui::ui_transform::UiGlobalTransform;
use bevy::window::WindowCloseRequested;
use bevy_noodle::prelude::*;

use super::dialog::open_menu;
use super::palette::Palette;
use crate::canvas::{EditorCanvas, NodeKey, PinKey, link_of, window_to_graph};
use crate::doc::connection_error;
use crate::state::{Action, Actions, Editor, ViewRequest};

pub struct InputPlugin;

impl Plugin for InputPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<RightPress>()
            .add_systems(Update, (shortcuts.run_if(not_typing), close_requested, draw_minimap))
            .add_observer(remember_right_press)
            .add_observer(middle_pan)
            .add_observer(canvas_click)
            .add_observer(edge_click)
            .add_observer(pin_click)
            .add_observer(wire_dropped);
    }
}

/// Shortcuts are off while a text field has the focus or a dialog is open.
pub fn not_typing(
    focus: Res<InputFocus>,
    fields: Query<(), With<EditableText>>,
    editor: Res<Editor>,
    palette: Res<Palette>,
) -> bool {
    editor.dialog.is_none() && !palette.open && focus.get().is_none_or(|f| !fields.contains(f))
}

/// Graph position under the cursor, if it is over the canvas.
fn cursor_graph(
    window: &Query<&Window>,
    ec: &EditorCanvas,
    canvases: &Query<(&ComputedNode, &UiGlobalTransform, &CanvasView)>,
) -> Option<(Vec2, Vec2)> {
    let cursor = window.iter().next()?.cursor_position()?;
    let (c, t, v) = canvases.get(ec.canvas).ok()?;
    let n = c.normalize_point(*t, cursor / c.inverse_scale_factor())?;
    if n.x.abs() > 0.5 || n.y.abs() > 0.5 {
        return None;
    }
    Some((cursor, window_to_graph(cursor, c, t, v)?))
}

#[allow(clippy::too_many_arguments)]
fn shortcuts(
    keys: Res<ButtonInput<KeyCode>>,
    window: Query<&Window>,
    ec: Option<Res<EditorCanvas>>,
    canvases: Query<(&ComputedNode, &UiGlobalTransform, &CanvasView)>,
    mut palette: ResMut<Palette>,
    mut actions: ResMut<Actions>,
) {
    use KeyCode::*;
    let ctrl = keys.any_pressed([ControlLeft, ControlRight, SuperLeft, SuperRight]);
    let shift = keys.any_pressed([ShiftLeft, ShiftRight]);
    let at = ec.as_ref().and_then(|ec| cursor_graph(&window, ec, &canvases));
    let mut push = |a: Action| actions.push(a);
    if ctrl {
        match () {
            _ if keys.just_pressed(KeyS) => push(if shift { Action::SaveAll } else { Action::Save }),
            _ if keys.just_pressed(KeyZ) => push(if shift { Action::Redo } else { Action::Undo }),
            _ if keys.just_pressed(KeyY) => push(Action::Redo),
            _ if keys.just_pressed(KeyC) => push(Action::Copy),
            _ if keys.just_pressed(KeyX) => push(Action::Cut),
            _ if keys.just_pressed(KeyV) => push(Action::Paste(at.map(|(_, g)| (g.x, g.y)))),
            _ if keys.just_pressed(KeyD) => push(Action::Duplicate),
            _ if keys.just_pressed(KeyW) => {
                // Resolved against the active tab when applied.
                push(Action::CloseTab(usize::MAX));
            }
            _ if keys.just_pressed(Tab) => push(Action::NextTab),
            _ if keys.just_pressed(KeyO) => push(Action::OpenProjectDialog),
            _ if keys.just_pressed(KeyL) => push(Action::AutoLayout),
            _ if keys.just_pressed(KeyQ) => push(Action::Quit),
            _ => {}
        }
        return;
    }
    match () {
        _ if keys.just_pressed(Delete) || keys.just_pressed(Backspace) => push(Action::DeleteSelection),
        _ if keys.just_pressed(KeyF) => push(Action::View(ViewRequest::FitSelection)),
        _ if keys.just_pressed(Home) => push(Action::View(ViewRequest::FitAll)),
        _ if keys.just_pressed(KeyC) => push(Action::AddComment(at.map(|(_, g)| (g.x, g.y)))),
        _ if keys.just_pressed(Tab) => {
            if let Some((screen, graph)) = at {
                palette.show(screen, graph, None);
            }
        }
        _ => {}
    }
}

/// UE: hold a key and click empty canvas to drop a node there.
fn hotkey_node(keys: &ButtonInput<KeyCode>) -> Option<&'static str> {
    use KeyCode::*;
    [
        (Digit1, "stdlib::Constant"),
        (Digit2, "stdlib::Constant2Vector"),
        (Digit3, "stdlib::Constant3Vector"),
        (Digit4, "stdlib::Constant4Vector"),
        (KeyS, "param::Scalar"),
        (KeyV, "param::Vector"),
        (KeyT, "stdlib::TextureSample"),
        (KeyM, "stdlib::Multiply"),
        (KeyA, "stdlib::Add"),
        (KeyL, "stdlib::Lerp"),
        (KeyU, "stdlib::TextureCoordinate"),
        (KeyR, "stdlib::Reroute"),
    ]
    .into_iter()
    .find(|(k, _)| keys.pressed(*k))
    .map(|(_, n)| n)
}

/// How far (logical px) the pointer may move between RMB press and release for it to count as a click
/// (palette, wire removal) rather than a pan.
const CLICK_SLOP: f32 = 3.0;

/// Where the right button was last pressed.
#[derive(Resource, Default)]
struct RightPress(Option<Vec2>);

impl RightPress {
    fn is_click(&self, release: Vec2) -> bool {
        self.0.is_some_and(|p| p.distance(release) <= CLICK_SLOP)
    }
}

fn remember_right_press(press: On<PointerPress>, mut right: ResMut<RightPress>) {
    if press.button == PointerButton::Secondary {
        right.0 = Some(press.pointer.position);
    }
}

/// MMB drag pans too (bevy_noodle has one pan button, set to RMB). Drags bubble from nodes up to the
/// canvas; edges have no parent, so their drags are taken where they start.
fn middle_pan(
    drag: On<PointerDrag>,
    ec: Option<Res<EditorCanvas>>,
    graph: GraphQuery,
    mut views: Query<&mut CanvasView>,
) {
    let Some(ec) = ec else { return };
    let target = drag.event_target();
    let on_edge = graph.edge_ports(target).is_some_and(|p| graph.canvas_of(p.input) == Some(ec.canvas));
    if drag.button != PointerButton::Middle || !(target == ec.canvas || on_edge) {
        return;
    }
    if let Ok(mut view) = views.get_mut(ec.canvas) {
        view.pan += drag.delta;
    }
}

#[allow(clippy::too_many_arguments)]
fn canvas_click(
    click: On<PointerClick>,
    ec: Option<Res<EditorCanvas>>,
    graph: GraphQuery,
    canvases: Query<(&ComputedNode, &UiGlobalTransform, &CanvasView)>,
    keys: Res<ButtonInput<KeyCode>>,
    editor: Res<Editor>,
    right: Res<RightPress>,
    mut palette: ResMut<Palette>,
    mut actions: ResMut<Actions>,
) {
    let Some(ec) = ec else { return };
    if click.event_target() != ec.canvas || editor.doc().is_none() {
        return;
    }
    if graph.node_of(click.original_event_target()).is_some() {
        return;
    }
    let Ok((c, t, v)) = canvases.get(ec.canvas) else { return };
    let Some(at) = window_to_graph(click.pointer.position, c, t, v) else { return };
    match click.button {
        PointerButton::Secondary if right.is_click(click.pointer.position) => {
            palette.show(click.pointer.position, at, None)
        }
        PointerButton::Primary => {
            if let Some(kind) = hotkey_node(&keys)
                && editor.asset_kind(&editor.doc().expect("checked").id).is_some()
            {
                actions.push(Action::AddNode { kind: kind.into(), pos: (at.x, at.y), link: None, params: Vec::new() });
            }
        }
        _ => {}
    }
}

/// Right click (not a pan) on an input pin opens its menu (type of polymorphic inputs, reset, disconnect).
fn pin_click(
    click: On<PointerClick>,
    keys: Query<&PinKey>,
    editor: Res<Editor>,
    right: Res<RightPress>,
    mut commands: Commands,
) {
    let Ok(key) = keys.get(click.event_target()) else { return };
    if key.output || click.button != PointerButton::Secondary || !right.is_click(click.pointer.position) {
        return;
    }
    let items = editor.pin_menu(&key.node, &key.pin);
    if !items.is_empty() {
        open_menu(&mut commands, click.pointer.position, items);
    }
}

/// Right click (not a pan) on a wire removes it; double click inserts a reroute point.
fn edge_click(
    click: On<PointerClick>,
    ec: Option<Res<EditorCanvas>>,
    graph: GraphQuery,
    keys_q: Query<&PinKey>,
    canvases: Query<(&ComputedNode, &UiGlobalTransform, &CanvasView)>,
    right: Res<RightPress>,
    mut actions: ResMut<Actions>,
) {
    let Some(ec) = ec else { return };
    let edge = click.event_target();
    let Some(ports) = graph.edge_ports(edge) else { return };
    if graph.canvas_of(ports.input) != Some(ec.canvas) {
        return;
    }
    let (Ok(a), Ok(b)) = (keys_q.get(ports.output), keys_q.get(ports.input)) else { return };
    match click.button {
        PointerButton::Secondary if right.is_click(click.pointer.position) => {
            actions.push(Action::Disconnect { to: (b.node.clone(), b.pin.clone()) })
        }
        PointerButton::Primary if click.count >= 2 => {
            let Ok((c, t, v)) = canvases.get(ec.canvas) else { return };
            if let Some(at) = window_to_graph(click.pointer.position, c, t, v) {
                actions.push(Action::AddReroute {
                    from: (a.node.clone(), a.pin.clone()),
                    to: (b.node.clone(), b.pin.clone()),
                    pos: (at.x - 10.0, at.y - 10.0),
                });
            }
        }
        _ => {}
    }
}

/// A wire dropped on empty space opens the palette filtered by its type (auto-connect on pick).
/// Dropped onto an incompatible port: say why (E002/E003) instead of opening the palette.
#[allow(clippy::too_many_arguments)]
fn wire_dropped(
    ev: On<WireDropped>,
    ec: Option<Res<EditorCanvas>>,
    keys_q: Query<&PinKey>,
    window: Query<&Window>,
    hover: Res<HoverMap>,
    editor: Res<Editor>,
    mut palette: ResMut<Palette>,
    mut actions: ResMut<Actions>,
) {
    let Some(ec) = ec else { return };
    if ev.canvas != ec.canvas {
        return;
    }
    let Ok(key) = keys_q.get(ev.from) else { return };
    let target = hover.values().flat_map(|m| m.keys()).find_map(|e| keys_q.get(*e).ok().filter(|k| *k != key));
    if let (Some(t), Some(d)) = (target, editor.doc()) {
        let (from, to) = if key.output { (key, t) } else { (t, key) };
        let reason = if from.output && !to.output {
            connection_error(
                &d.asset,
                (&from.node, &from.pin),
                (&to.node, &to.pin),
                &editor.registry,
                &editor.functions,
                &d.id.path,
            )
        } else {
            Some("an output must connect to an input".into())
        };
        let reason = reason.unwrap_or_else(|| "connection refused".into());
        actions.push(Action::Notify(format!("Connection refused: {reason}")));
        return;
    }
    let screen = window.iter().next().and_then(|w| w.cursor_position()).unwrap_or(Vec2::splat(200.0));
    palette.show(screen, ev.position, Some(link_of(key)));
}

fn close_requested(mut ev: MessageReader<WindowCloseRequested>, mut actions: ResMut<Actions>) {
    if ev.read().count() > 0 {
        actions.push(Action::Quit);
    }
}

// ---------------------------------------------------------------------------------------------
// Minimap (bottom-left overlay of the canvas)

const MAP: Vec2 = Vec2::new(200.0, 130.0);

#[derive(Component, Default, Clone)]
pub struct Minimap;

#[derive(Component, Default, Clone)]
pub struct MiniView;

#[derive(Component)]
pub struct MiniNode(Entity);

pub fn minimap() -> impl Scene {
    bsn! {
        Minimap
        Node {
            position_type: PositionType::Absolute,
            left: px(10),
            bottom: px(10),
            width: px(MAP.x),
            height: px(MAP.y),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(4)),
            overflow: Overflow::clip(),
        }
        BackgroundColor(Color::srgba(0.08, 0.09, 0.11, 0.85))
        BorderColor::all(Color::srgba(1.0, 1.0, 1.0, 0.15))
        on(look_here::<PointerPress>)
        on(look_here::<PointerDrag>)
        Children [
            MiniView
            Node { position_type: PositionType::Absolute, border: UiRect::all(px(1.5)) }
            BorderColor::all(Color::srgb_u8(250, 204, 92))
            Pickable::IGNORE
        ]
    }
}

#[derive(Clone, Copy)]
struct Mapping {
    origin: Vec2,
    scale: f32,
    offset: Vec2,
}

impl Mapping {
    fn new(rects: impl Iterator<Item = Rect>) -> Option<Self> {
        let bounds = rects.reduce(|a, b| a.union(b))?.inflate(80.0);
        let scale = (MAP / bounds.size()).min_element();
        Some(Mapping { origin: bounds.min, scale, offset: (MAP - bounds.size() * scale) / 2.0 })
    }

    fn to_map(self, p: Vec2) -> Vec2 {
        (p - self.origin) * self.scale + self.offset
    }

    fn to_graph(self, p: Vec2) -> Vec2 {
        (p - self.offset) / self.scale + self.origin
    }
}

fn node_rect(p: &NodePosition, c: &ComputedNode) -> Rect {
    Rect::from_corners(p.0, p.0 + c.size() * c.inverse_scale_factor())
}

#[allow(clippy::type_complexity)]
fn draw_minimap(
    mut commands: Commands,
    ec: Option<Res<EditorCanvas>>,
    minimap: Query<(Entity, &mut Visibility), With<Minimap>>,
    views: Query<(&CanvasView, &ComputedNode), Without<GraphNode>>,
    nodes: Query<(Entity, &NodePosition, &ComputedNode), With<NodeKey>>,
    mut minis: Query<(Entity, &MiniNode, &mut Node), Without<MiniView>>,
    mut view_rect: Query<&mut Node, With<MiniView>>,
) {
    let Some(ec) = ec else { return };
    let mut minimap = minimap;
    let Ok((map_e, mut vis)) = minimap.single_mut() else { return };
    let mapping = Mapping::new(nodes.iter().map(|(_, p, c)| node_rect(p, c)));
    let want = if mapping.is_some() { Visibility::Inherited } else { Visibility::Hidden };
    vis.set_if_neq(want);
    let (Some(map), Ok((view, canvas))) = (mapping, views.get(ec.canvas)) else { return };
    let place = |node: &mut Node, rect: Rect| {
        let r = Rect::from_corners(map.to_map(rect.min), map.to_map(rect.max));
        (node.left, node.top) = (px(r.min.x), px(r.min.y));
        (node.width, node.height) = (px(r.width().max(1.0)), px(r.height().max(1.0)));
    };
    for (e, mini, mut node) in &mut minis {
        match nodes.get(mini.0) {
            Ok((_, p, c)) => place(&mut node, node_rect(p, c)),
            Err(_) => commands.entity(e).try_despawn(),
        }
    }
    for (n, ..) in nodes.iter().filter(|(n, ..)| !minis.iter().any(|m| m.1.0 == *n)) {
        commands.spawn((
            MiniNode(n),
            Node { position_type: PositionType::Absolute, ..default() },
            BackgroundColor(Color::srgba(0.4, 0.6, 0.95, 0.6)),
            Pickable::IGNORE,
            ChildOf(map_e),
        ));
    }
    if let Ok(mut vr) = view_rect.single_mut() {
        let size = canvas.size() * canvas.inverse_scale_factor();
        place(&mut vr, Rect::from_corners(view.canvas_to_graph(Vec2::ZERO), view.canvas_to_graph(size)));
    }
}

fn look_here<E: EntityEvent + bevy::picking::events::PointerEvent>(
    ev: On<E>,
    ec: Option<Res<EditorCanvas>>,
    maps: Query<(&ComputedNode, &UiGlobalTransform), With<Minimap>>,
    mut views: Query<(&mut CanvasView, &ComputedNode), Without<Minimap>>,
    nodes: Query<(&NodePosition, &ComputedNode), With<NodeKey>>,
) {
    let Some(ec) = ec else { return };
    let (Ok((map, t)), Some(mapping)) =
        (maps.get(ev.event_target()), Mapping::new(nodes.iter().map(|(p, c)| node_rect(p, c))))
    else {
        return;
    };
    let Some(n) = map.normalize_point(*t, ev.pointer().position / map.inverse_scale_factor()) else { return };
    let target = mapping.to_graph((n + 0.5) * MAP);
    if let Ok((mut view, canvas)) = views.get_mut(ec.canvas) {
        let size = canvas.size() * canvas.inverse_scale_factor();
        view.pan = size / 2.0 - target * view.zoom;
    }
}
