//! Canvas gestures (spec §9, UE-like): clicks, held key + click node placement, wire drops, and the minimap.
//! Hotkeys are the editor kinds' input contexts (`editors::main_input`, decision 0032).

use bevy::ecs::system::SystemParam;
use bevy::picking::hover::HoverMap;
use bevy::prelude::*;
use bevy::ui::ui_transform::UiGlobalTransform;
use bevy::window::WindowCloseRequested;
use bevy_enhanced_input::prelude::ModKeys;
use bevy_noodle::prelude::*;

use super::dialog::open_menu;
use super::palette::Palette;
use super::props::PropField;
use super::tree::TreeRow;
use crate::canvas::{EditorCanvas, NodeKey, PinKey, link_of, window_to_graph};
use crate::doc::connection_error;
use crate::droppable::Dropped;
use crate::editors::main_input::{mods, no_command_mods};
use crate::state::{Action, Actions, Editor};

pub struct InputPlugin;

impl Plugin for InputPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<RightPress>()
            .init_resource::<PreviewKey>()
            .add_systems(Update, (close_requested, draw_minimap))
            .add_observer(remember_right_press)
            .add_observer(middle_pan)
            .add_observer(canvas_click)
            .add_observer(edge_click)
            .add_observer(pin_click)
            .add_observer(node_click)
            .add_observer(wire_dropped)
            .add_observer(drop_on_canvas);
    }
}

/// Graph position under the cursor, if it is over the canvas.
fn cursor_graph(
    window: &Query<&Window>,
    ec: &EditorCanvas,
    canvases: &Query<(&ComputedNode, &UiGlobalTransform, &CanvasView)>,
    ui_scale: f32,
) -> Option<(Vec2, Vec2)> {
    let cursor = window.iter().next()?.cursor_position()?;
    let (c, t, v) = canvases.get(ec.canvas).ok()?;
    let n = c.normalize_point(*t, cursor / (c.inverse_scale_factor() * ui_scale))?;
    if n.x.abs() > 0.5 || n.y.abs() > 0.5 {
        return None;
    }
    Some((cursor, window_to_graph(cursor, c, t, v, ui_scale)?))
}

/// Where the cursor is over the graph canvas: for hotkeys acting at the cursor (paste, comment, palette).
#[derive(SystemParam)]
pub struct CanvasCursor<'w, 's> {
    window: Query<'w, 's, &'static Window>,
    ec: Option<Res<'w, EditorCanvas>>,
    canvases: Query<'w, 's, (&'static ComputedNode, &'static UiGlobalTransform, &'static CanvasView)>,
    ui_scale: Option<Res<'w, UiScale>>,
}

impl CanvasCursor<'_, '_> {
    /// Window and graph position of the cursor, if it is over the canvas.
    pub fn at(&self) -> Option<(Vec2, Vec2)> {
        let ui_scale = self.ui_scale.as_ref().map_or(1.0, |s| s.0);
        cursor_graph(&self.window, self.ec.as_deref()?, &self.canvases, ui_scale)
    }

    /// Graph position of the cursor, as the actions take it.
    pub fn graph_pos(&self) -> Option<(f32, f32)> {
        self.at().map(|(_, g)| (g.x, g.y))
    }
}

/// `V` toggles the node preview on release, unless `V` + click dropped a `param::Vector` meanwhile
/// (decision 0021).
#[derive(Resource, Default)]
pub(crate) struct PreviewKey {
    held: bool,
    placed: bool,
}

impl PreviewKey {
    pub(crate) fn press(&mut self) {
        *self = Self { held: true, placed: false };
    }

    fn placed(&mut self) {
        self.placed = self.held;
    }

    /// True when the release toggles the preview.
    pub(crate) fn release(&mut self) -> bool {
        let toggle = self.held && !self.placed;
        *self = Self::default();
        toggle
    }
}

/// UE: hold a key and click empty canvas to drop a node there (decision 0032).
const NODE_HOTKEYS: [(KeyCode, &str); 18] = {
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
        (KeyD, "stdlib::Divide"),
        (KeyO, "stdlib::OneMinus"),
        (KeyE, "stdlib::Power"),
        (KeyN, "stdlib::Normalize"),
        (KeyP, "stdlib::Panner"),
        (KeyI, "stdlib::If"),
    ]
};

/// The node a held plain key places; none while Ctrl, Alt or Super is held (decision 0032).
fn hotkey_node(keys: &ButtonInput<KeyCode>) -> Option<&'static str> {
    if !no_command_mods(keys) {
        return None;
    }
    NODE_HOTKEYS.into_iter().find(|(k, _)| keys.pressed(*k)).map(|(_, n)| n)
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
    ui_scale: Res<UiScale>,
    keys: Res<ButtonInput<KeyCode>>,
    editor: Res<Editor>,
    right: Res<RightPress>,
    mut palette: ResMut<Palette>,
    mut preview: ResMut<PreviewKey>,
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
    let Some(at) = window_to_graph(click.pointer.position, c, t, v, ui_scale.0) else { return };
    match click.button {
        PointerButton::Secondary if right.is_click(click.pointer.position) => {
            palette.show(click.pointer.position, at, None)
        }
        PointerButton::Primary => {
            if let Some(kind) = hotkey_node(&keys)
                && editor.asset_kind(&editor.doc().expect("checked").id).is_some()
            {
                if keys.pressed(KeyCode::KeyV) {
                    preview.placed();
                }
                actions.push(Action::AddNode { kind: kind.into(), pos: (at.x, at.y), link: None, params: Vec::new() });
            }
        }
        _ => {}
    }
}

/// Alt held: UE's "break" modifier (Alt+LMB on a pin or a wire).
fn alt(keys: &ButtonInput<KeyCode>) -> bool {
    mods(keys).contains(ModKeys::ALT)
}

/// Ctrl (or Super, as in the hotkeys) held: a wire drag moves the pin's wires (decision 0032).
fn ctrl(keys: &ButtonInput<KeyCode>) -> bool {
    mods(keys).intersects(ModKeys::CONTROL | ModKeys::SUPER)
}

/// Right click (not a pan) on an input pin opens its menu (type of polymorphic inputs, reset, disconnect);
/// Alt+LMB on any pin removes its wires (UE).
fn pin_click(
    click: On<PointerClick>,
    pins: Query<&PinKey>,
    keys: Res<ButtonInput<KeyCode>>,
    editor: Res<Editor>,
    right: Res<RightPress>,
    mut actions: ResMut<Actions>,
    mut commands: Commands,
) {
    let Ok(key) = pins.get(click.event_target()) else { return };
    match click.button {
        PointerButton::Primary if alt(&keys) => {
            actions.push(Action::DisconnectPin { node: key.node.clone(), pin: key.pin.clone(), output: key.output })
        }
        PointerButton::Secondary if !key.output && right.is_click(click.pointer.position) => {
            let items = editor.pin_menu(&key.node, &key.pin);
            if !items.is_empty() {
                open_menu(&mut commands, click.pointer.position, items);
            }
        }
        _ => {}
    }
}

/// Right click (not a pan) on a node body opens the node menu (decision 0021); an unselected node is selected
/// first, so the generic rows (copy, duplicate, delete) act on it. A double click on the body opens the file the
/// node refers to (function, curve, constant table; decision 0023).
#[allow(clippy::too_many_arguments)]
fn node_click(
    click: On<PointerClick>,
    ec: Option<Res<EditorCanvas>>,
    nodes: Query<&NodeKey>,
    pins: Query<(), With<PinKey>>,
    fields: Query<(), With<PropField>>,
    parents: Query<&ChildOf>,
    editor: Res<Editor>,
    right: Res<RightPress>,
    mut actions: ResMut<Actions>,
    mut commands: Commands,
) {
    let Ok(NodeKey(node)) = nodes.get(click.event_target()) else { return };
    if pins.contains(click.original_event_target()) {
        return;
    }
    if click.button == PointerButton::Primary && click.count >= 2 {
        // Inline editors inside the node keep their own clicks.
        let on_field = std::iter::once(click.original_event_target())
            .chain(parents.iter_ancestors(click.original_event_target()))
            .take_while(|e| *e != click.event_target())
            .any(|e| fields.contains(e));
        if !on_field && let Some(id) = editor.referenced_asset(node) {
            actions.push(Action::OpenAsset(id));
        }
        return;
    }
    if click.button != PointerButton::Secondary || !right.is_click(click.pointer.position) {
        return;
    }
    let Some(d) = editor.doc() else { return };
    if !d.selection.contains(node) {
        // On the canvas too: `sync_selection` copies the canvas selection into the model every frame.
        if let Some(ec) = ec {
            commands.select(ec.canvas, vec![click.event_target()], SelectMode::Replace);
        }
        actions.push(Action::Select([node.clone()].into()));
    }
    open_menu(&mut commands, click.pointer.position, editor.node_menu(node));
}

/// A sidebar row (a game image, a Texture asset, a built-in texture) dragged onto the graph canvas (design §4.3).
/// `PointerDragDrop` bubbles from the entity under the pointer (a node, its pins and fields, or the canvas itself) up
/// to the canvas; what the drop does is decided by [`Action::Drop`].
#[allow(clippy::too_many_arguments)]
fn drop_on_canvas(
    ev: On<PointerDragDrop>,
    ec: Option<Res<EditorCanvas>>,
    rows: Query<&TreeRow>,
    graph: GraphQuery,
    nodes: Query<&NodeKey>,
    canvases: Query<(&ComputedNode, &UiGlobalTransform, &CanvasView)>,
    ui_scale: Res<UiScale>,
    editor: Res<Editor>,
    mut actions: ResMut<Actions>,
) {
    let Some(ec) = ec else { return };
    if ev.event_target() != ec.canvas || ev.button != PointerButton::Primary || editor.doc().is_none() {
        return;
    }
    let Some(item) = rows.get(ev.dropped).ok().and_then(Dropped::from_row) else { return };
    let node = graph.node_of(ev.original_event_target()).and_then(|n| nodes.get(n).ok()).map(|k| k.0.clone());
    let Ok((c, t, v)) = canvases.get(ec.canvas) else { return };
    let Some(at) = window_to_graph(ev.pointer.position, c, t, v, ui_scale.0) else { return };
    actions.push(Action::Drop { item, pos: (at.x, at.y), node });
}

/// Right click (not a pan) or Alt+LMB on a wire removes it (UE); double click inserts a reroute point.
#[allow(clippy::too_many_arguments)]
fn edge_click(
    click: On<PointerClick>,
    ec: Option<Res<EditorCanvas>>,
    graph: GraphQuery,
    keys_q: Query<&PinKey>,
    canvases: Query<(&ComputedNode, &UiGlobalTransform, &CanvasView)>,
    ui_scale: Res<UiScale>,
    keys: Res<ButtonInput<KeyCode>>,
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
    let remove = match click.button {
        PointerButton::Secondary => right.is_click(click.pointer.position),
        PointerButton::Primary => alt(&keys),
        PointerButton::Middle => false,
    };
    match click.button {
        _ if remove => actions.push(Action::Disconnect { to: (b.node.clone(), b.pin.clone()) }),
        PointerButton::Primary if click.count >= 2 => {
            let Ok((c, t, v)) = canvases.get(ec.canvas) else { return };
            if let Some(at) = window_to_graph(click.pointer.position, c, t, v, ui_scale.0) {
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
/// With Ctrl held (UE "move wires", decision 0032) a drop on another pin of the same direction moves the wires of
/// the pin the drag started from there (bevy_noodle has no wire target on such a pin, so the drop lands here);
/// a drop on empty space does nothing.
#[allow(clippy::too_many_arguments)]
fn wire_dropped(
    ev: On<WireDropped>,
    ec: Option<Res<EditorCanvas>>,
    keys_q: Query<&PinKey>,
    window: Query<&Window>,
    hover: Res<HoverMap>,
    keys: Res<ButtonInput<KeyCode>>,
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
    let moving = ctrl(&keys);
    match target {
        Some(t) if moving && t.output == key.output => {
            return actions.push(Action::MoveWires {
                node: key.node.clone(),
                pin: key.pin.clone(),
                output: key.output,
                to: (t.node.clone(), t.pin.clone()),
            });
        }
        None if moving => return,
        _ => {}
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hotkey_nodes_exist() {
        let registry = shader_stdlib::registry();
        for (key, kind) in NODE_HOTKEYS {
            assert!(registry.get(kind).is_some(), "{key:?} drops `{kind}`, which is not registered");
        }
        // UE keys we have nodes for (no BumpOffset in the stdlib).
        let ue = [
            (KeyCode::KeyD, "stdlib::Divide"),
            (KeyCode::KeyO, "stdlib::OneMinus"),
            (KeyCode::KeyE, "stdlib::Power"),
            (KeyCode::KeyN, "stdlib::Normalize"),
            (KeyCode::KeyP, "stdlib::Panner"),
            (KeyCode::KeyI, "stdlib::If"),
        ];
        for (key, kind) in ue {
            let mut keys = ButtonInput::<KeyCode>::default();
            keys.press(key);
            assert_eq!(hotkey_node(&keys), Some(kind), "{key:?}");
        }
        let keys: Vec<KeyCode> = NODE_HOTKEYS.iter().map(|(k, _)| *k).collect();
        assert!(keys.iter().all(|k| keys.iter().filter(|o| *o == k).count() == 1), "one node per key");
        // Plain keys only (decision 0032): Ctrl+D, Alt+N or Super+A held place nothing; Shift is allowed.
        for m in [KeyCode::ControlLeft, KeyCode::AltRight, KeyCode::SuperLeft] {
            for key in [KeyCode::KeyD, KeyCode::KeyN, KeyCode::KeyA] {
                let mut keys = ButtonInput::<KeyCode>::default();
                keys.press(m);
                keys.press(key);
                assert_eq!(hotkey_node(&keys), None, "{m:?}+{key:?}");
            }
        }
        let mut keys = ButtonInput::<KeyCode>::default();
        keys.press(KeyCode::ShiftLeft);
        keys.press(KeyCode::KeyD);
        assert_eq!(hotkey_node(&keys), Some("stdlib::Divide"));
    }

    #[test]
    fn preview_key_toggles_on_release_unless_a_node_was_placed() {
        let mut k = PreviewKey::default();
        k.press();
        assert!(k.release());
        k.press();
        k.placed();
        assert!(!k.release());
        assert!(!k.release(), "release without press");
    }
}
