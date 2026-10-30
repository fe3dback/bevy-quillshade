//! Canvas gestures (spec §9, UE-like): clicks, held key + click node placement, wire drops, tree drops and the
//! minimap. Hotkeys are the graph editor's input context ([`crate::editor`], decision 0032).

use std::marker::PhantomData;

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy::ui::ui_transform::UiGlobalTransform;
use bevy_enhanced_input::prelude::ModKeys;
use bevy_noodle::prelude::*;
use editor_core::action::{Action, Actions};
use editor_core::keys::{mods, no_command_mods};
use editor_core::menu::MenuEntry;
use editor_core::state::editor::Editor;
use editor_core::ui::dialog::open_icon_menu;
use editor_core::ui::props::PropField;
use editor_core::ui::tree::TreeRow;

use crate::canvas::{EditorCanvas, FrameKey, FrameTitleStrip, GraphOf, NodeHeader, NodeKey, window_to_graph};
use crate::domain::{GraphCtxRef, GraphDomain, active};
use crate::edit::{GraphEdit, PinKey};
use crate::palette::Palette;
use crate::view::PlaceSpec;

/// The canvas gestures of `G` (the window app; UI tests add it themselves).
pub struct InputPlugin<G>(PhantomData<fn() -> G>);

impl<G> Default for InputPlugin<G> {
    fn default() -> Self {
        InputPlugin(PhantomData)
    }
}

impl<G: GraphDomain> Plugin for InputPlugin<G> {
    fn build(&self, app: &mut App) {
        app.init_resource::<RightPress>()
            .add_systems(Update, draw_minimap::<G>)
            .add_observer(remember_right_press)
            .add_observer(middle_pan::<G>)
            .add_observer(canvas_click::<G>)
            .add_observer(edge_click::<G>)
            .add_observer(pin_click::<G>)
            .add_observer(node_click::<G>)
            .add_observer(frame_click::<G>)
            .add_observer(wire_dropped::<G>)
            .add_observer(lifted_wires_dropped::<G>)
            .add_observer(drop_on_canvas::<G>);
    }
}

/// Graph position under the cursor, if it is over the canvas.
fn cursor_graph(
    window: &Query<&Window>,
    canvas: Entity,
    canvases: &Query<(&ComputedNode, &UiGlobalTransform, &CanvasView)>,
    ui_scale: f32,
) -> Option<(Vec2, Vec2)> {
    let cursor = window.iter().next()?.cursor_position()?;
    let (c, t, v) = canvases.get(canvas).ok()?;
    let n = c.normalize_point(*t, cursor / (c.inverse_scale_factor() * ui_scale))?;
    if n.x.abs() > 0.5 || n.y.abs() > 0.5 {
        return None;
    }
    Some((cursor, window_to_graph(cursor, c, t, v, ui_scale)?))
}

/// Where the cursor is over the graph canvas of `G`: for hotkeys acting at the cursor (paste, comment, palette).
#[derive(SystemParam)]
pub struct CanvasCursor<'w, 's, G: GraphDomain> {
    window: Query<'w, 's, &'static Window>,
    ec: Option<Res<'w, EditorCanvas<G>>>,
    canvases: Query<'w, 's, (&'static ComputedNode, &'static UiGlobalTransform, &'static CanvasView)>,
    ui_scale: Option<Res<'w, UiScale>>,
}

impl<G: GraphDomain> CanvasCursor<'_, '_, G> {
    /// Window and graph position of the cursor, if it is over the canvas.
    pub fn at(&self) -> Option<(Vec2, Vec2)> {
        let ui_scale = self.ui_scale.as_ref().map_or(1.0, |s| s.0);
        cursor_graph(&self.window, self.ec.as_deref()?.canvas, &self.canvases, ui_scale)
    }

    /// Graph position of the cursor, as the actions take it.
    pub fn graph_pos(&self) -> Option<(f32, f32)> {
        self.at().map(|(_, g)| (g.x, g.y))
    }
}

/// The node a held plain key places (`table`: [`GraphDomain::placement_hotkeys`]); none while Ctrl, Alt or Super is
/// held (decision 0032), so Ctrl+1..3 (View) drop nothing. Ctrl released while 1 is still held, then a click, does
/// drop a Constant: that is "hold 1 + click".
pub fn hotkey_node(keys: &ButtonInput<KeyCode>, table: &[(KeyCode, PlaceSpec)]) -> Option<PlaceSpec> {
    if !no_command_mods(keys) {
        return None;
    }
    table.iter().find(|(k, _)| keys.pressed(*k)).map(|(_, n)| *n)
}

/// A node was placed with a held key on `G`'s canvas (decision 0032). A domain that gives a placement key a second
/// meaning (Material: V previews on release) observes it.
#[derive(Event)]
pub struct NodePlaced<G>(PhantomData<fn() -> G>);

impl<G> Default for NodePlaced<G> {
    fn default() -> Self {
        NodePlaced(PhantomData)
    }
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
fn middle_pan<G: GraphDomain>(
    drag: On<PointerDrag>,
    ec: Option<Res<EditorCanvas<G>>>,
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
fn canvas_click<G: GraphDomain>(
    click: On<PointerClick>,
    ec: Option<Res<EditorCanvas<G>>>,
    graph: GraphQuery,
    canvases: Query<(&ComputedNode, &UiGlobalTransform, &CanvasView)>,
    ui_scale: Res<UiScale>,
    keys: Res<ButtonInput<KeyCode>>,
    editor: Res<Editor>,
    right: Res<RightPress>,
    mut palette: ResMut<Palette<G>>,
    mut actions: ResMut<Actions>,
    mut commands: Commands,
) {
    let Some(ec) = ec else { return };
    let Some(d) = active::<G>(&editor) else { return };
    if click.event_target() != ec.canvas {
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
            if let Some(spec) = hotkey_node(&keys, G::placement_hotkeys())
                && let Some(a) = G::place(&GraphCtxRef { editor: &editor }, d, spec, at)
            {
                commands.trigger(NodePlaced::<G>::default());
                actions.push(a);
            }
        }
        _ => {}
    }
}

/// Alt held: UE's "break" modifier (Alt+LMB on a pin or a wire).
fn alt(keys: &ButtonInput<KeyCode>) -> bool {
    mods(keys).contains(ModKeys::ALT)
}

/// A menu at `pos` from the domain's entries.
fn open_entries(commands: &mut Commands, pos: Vec2, items: Vec<MenuEntry>) {
    open_icon_menu(commands, pos, items.into_iter().map(|e| (e.icon, e.label, e.action)).collect());
}

/// Right click (not a pan) on an input pin opens its menu ([`GraphDomain::pin_menu`]); Alt+LMB on any pin removes
/// its wires (UE).
fn pin_click<G: GraphDomain>(
    click: On<PointerClick>,
    pins: Query<(&PinKey, &GraphOf)>,
    keys: Res<ButtonInput<KeyCode>>,
    editor: Res<Editor>,
    right: Res<RightPress>,
    mut actions: ResMut<Actions>,
    mut commands: Commands,
) {
    let Some((key, _)) = pins.get(click.event_target()).ok().filter(|(_, g)| g.0 == G::VIEW) else { return };
    let Some(d) = active::<G>(&editor) else { return };
    match click.button {
        PointerButton::Primary if alt(&keys) => {
            actions.push(GraphEdit::DisconnectPin { node: key.node.clone(), pin: key.pin.clone(), output: key.output })
        }
        PointerButton::Secondary if !key.output && right.is_click(click.pointer.position) => {
            let items = G::pin_menu(&GraphCtxRef { editor: &editor }, d, key);
            if !items.is_empty() {
                open_entries(&mut commands, click.pointer.position, items);
            }
        }
        _ => {}
    }
}

/// Right click (not a pan) on a node body opens the node menu (decision 0021); an unselected node is selected
/// first, so the generic rows (copy, duplicate, delete) act on it. A double click on the body runs
/// [`GraphDomain::open_action`] (open the file the node refers to); on the header [`GraphDomain::header_action`]
/// (design §5.5).
#[allow(clippy::too_many_arguments)]
fn node_click<G: GraphDomain>(
    click: On<PointerClick>,
    ec: Option<Res<EditorCanvas<G>>>,
    nodes: Query<(&NodeKey, &GraphOf)>,
    pins: Query<(), With<PinKey>>,
    (fields, headers): (Query<(), With<PropField>>, Query<(), With<NodeHeader>>),
    parents: Query<&ChildOf>,
    editor: Res<Editor>,
    right: Res<RightPress>,
    mut actions: ResMut<Actions>,
    mut commands: Commands,
) {
    let Some((NodeKey(node), _)) = nodes.get(click.event_target()).ok().filter(|(_, g)| g.0 == G::VIEW) else { return };
    if pins.contains(click.original_event_target()) {
        return;
    }
    let Some(d) = active::<G>(&editor) else { return };
    let ctx = GraphCtxRef { editor: &editor };
    if click.button == PointerButton::Primary && click.count >= 2 {
        // From the clicked entity up to the node.
        let path = || {
            std::iter::once(click.original_event_target())
                .chain(parents.iter_ancestors(click.original_event_target()))
                .take_while(|e| *e != click.event_target())
        };
        // Inline editors inside the node keep their own clicks.
        if path().any(|e| fields.contains(e)) {
            return;
        }
        let action = if path().any(|e| headers.contains(e)) {
            G::header_action(&ctx, d, node)
        } else {
            G::open_action(&ctx, d, node)
        };
        if let Some(a) = action {
            actions.push(a);
        }
        return;
    }
    if click.button != PointerButton::Secondary || !right.is_click(click.pointer.position) {
        return;
    }
    if !G::view(d).selection.contains(node) {
        // On the canvas too: `sync_selection` copies the canvas selection into the model every frame.
        if let Some(ec) = ec {
            commands.select(ec.canvas, vec![click.event_target()], SelectMode::Replace);
        }
        actions.push(GraphEdit::Select([node.clone()].into()));
    }
    open_entries(&mut commands, click.pointer.position, G::node_menu(&ctx, d, std::slice::from_ref(node)));
}

/// A double click on a comment frame's title strip edits its title (design §5.5).
fn frame_click<G: GraphDomain>(
    click: On<PointerClick>,
    frames: Query<(&FrameKey, &GraphOf)>,
    strips: Query<(), With<FrameTitleStrip>>,
    editor: Res<Editor>,
    mut actions: ResMut<Actions>,
) {
    let Some((FrameKey(id), _)) = frames.get(click.event_target()).ok().filter(|(_, g)| g.0 == G::VIEW) else { return };
    let on_title = strips.contains(click.original_event_target());
    if click.button == PointerButton::Primary && click.count >= 2 && on_title && active::<G>(&editor).is_some() {
        actions.push(GraphEdit::FrameTitle(id.clone()));
    }
}

/// A sidebar row dragged onto the graph canvas (design §4.3). `PointerDragDrop` bubbles from the entity under the
/// pointer (a node, its pins and fields, or the canvas itself) up to the canvas; what the drop does is decided by
/// [`GraphDomain::drop`].
#[allow(clippy::too_many_arguments)]
fn drop_on_canvas<G: GraphDomain>(
    ev: On<PointerDragDrop>,
    ec: Option<Res<EditorCanvas<G>>>,
    rows: Query<&TreeRow>,
    graph: GraphQuery,
    nodes: Query<&NodeKey>,
    canvases: Query<(&ComputedNode, &UiGlobalTransform, &CanvasView)>,
    ui_scale: Res<UiScale>,
    editor: Res<Editor>,
    mut actions: ResMut<Actions>,
) {
    let Some(ec) = ec else { return };
    let Some(d) = active::<G>(&editor) else { return };
    if ev.event_target() != ec.canvas || ev.button != PointerButton::Primary {
        return;
    }
    let Ok(row) = rows.get(ev.dropped) else { return };
    let node = graph.node_of(ev.original_event_target()).and_then(|n| nodes.get(n).ok()).map(|k| k.0.clone());
    let Ok((c, t, v)) = canvases.get(ec.canvas) else { return };
    let Some(at) = window_to_graph(ev.pointer.position, c, t, v, ui_scale.0) else { return };
    if let Some(a) = G::drop(&GraphCtxRef { editor: &editor }, d, row, at, node.as_deref()) {
        actions.push(a);
    }
}

/// Right click (not a pan) or Alt+LMB on a wire removes it (UE); double click inserts a reroute point.
#[allow(clippy::too_many_arguments)]
fn edge_click<G: GraphDomain>(
    click: On<PointerClick>,
    ec: Option<Res<EditorCanvas<G>>>,
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
        _ if remove => actions.push(GraphEdit::Disconnect { to: (b.node.clone(), b.pin.clone()) }),
        PointerButton::Primary if click.count >= 2 => {
            let Ok((c, t, v)) = canvases.get(ec.canvas) else { return };
            if let Some(at) = window_to_graph(click.pointer.position, c, t, v, ui_scale.0) {
                actions.push(GraphEdit::AddReroute {
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
/// Dropped onto an incompatible port (bevy_noodle names it, `WireDropped::onto`): say why
/// ([`GraphDomain::can_connect`]) instead of opening the palette. (A Ctrl+drag lifts the pin's wires instead of
/// dragging a new one, decision 0032: bevy_noodle's move gesture, `canvas::check_move`.)
fn wire_dropped<G: GraphDomain>(
    ev: On<WireDropped>,
    ec: Option<Res<EditorCanvas<G>>>,
    keys_q: Query<&PinKey>,
    window: Query<&Window>,
    editor: Res<Editor>,
    mut palette: ResMut<Palette<G>>,
    mut actions: ResMut<Actions>,
) {
    let Some(ec) = ec else { return };
    if ev.canvas != ec.canvas {
        return;
    }
    let Ok(key) = keys_q.get(ev.from) else { return };
    let target = ev.onto.and_then(|e| keys_q.get(e).ok());
    if let (Some(t), Some(d)) = (target, active::<G>(&editor)) {
        let (from, to) = if key.output { (key, t) } else { (t, key) };
        let reason = if from.output && !to.output {
            G::can_connect(&GraphCtxRef { editor: &editor }, d, from, to).err()
        } else {
            Some("an output must connect to an input".into())
        };
        let reason = reason.unwrap_or_else(|| "connection refused".into());
        actions.push(Action::Notify(format!("Connection refused: {reason}")));
        return;
    }
    let screen = window.iter().next().and_then(|w| w.cursor_position()).unwrap_or(Vec2::splat(200.0));
    palette.show(screen, ev.position, Some(key.clone()));
}

/// Wires a Ctrl+drag lifted, dropped where they cannot move (decision 0032, design §5.3): over a pin of the same
/// direction the move is refused for (bevy_noodle names it, `EdgesDropped::onto`), say why (the reason
/// [`GraphDomain::plan_move_wires`] gives); elsewhere nothing happens (no palette). The wires are back at their pin
/// either way.
fn lifted_wires_dropped<G: GraphDomain>(
    ev: On<EdgesDropped>,
    ec: Option<Res<EditorCanvas<G>>>,
    keys_q: Query<&PinKey>,
    editor: Res<Editor>,
    mut actions: ResMut<Actions>,
) {
    if ec.is_none_or(|ec| ev.canvas != ec.canvas) {
        return;
    }
    let Ok(key) = keys_q.get(ev.from) else { return };
    let Some(to) = ev.onto.and_then(|e| keys_q.get(e).ok()).filter(|t| t.output == key.output) else { return };
    let plan = match active::<G>(&editor) {
        Some(d) => G::plan_move_wires(&GraphCtxRef { editor: &editor }, d, key, to),
        None => Err("no graph is open".into()),
    };
    if let Err(why) = plan {
        actions.push(Action::Notify(why));
    }
}

// ---------------------------------------------------------------------------------------------
// Minimap (bottom-left overlay of the canvas)

const MAP: Vec2 = Vec2::new(200.0, 130.0);

/// The minimap of `G`'s canvas.
#[derive(Component)]
pub struct Minimap<G>(PhantomData<fn() -> G>);

impl<G> Default for Minimap<G> {
    fn default() -> Self {
        Minimap(PhantomData)
    }
}

impl<G> Clone for Minimap<G> {
    fn clone(&self) -> Self {
        Minimap::default()
    }
}

#[derive(Component, Default, Clone)]
pub struct MiniView;

#[derive(Component)]
pub struct MiniNode(Entity);

/// The minimap overlay of `G`'s canvas (`DomainRegistrar::doc_overlay`).
pub fn minimap<G: GraphDomain>() -> impl Scene {
    let marker = Minimap::<G>::default();
    bsn! {
        marker
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
        on(look_here::<G, PointerPress>)
        on(look_here::<G, PointerDrag>)
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
fn draw_minimap<G: GraphDomain>(
    mut commands: Commands,
    ec: Option<Res<EditorCanvas<G>>>,
    minimap: Query<(Entity, &mut Visibility), With<Minimap<G>>>,
    views: Query<(&CanvasView, &ComputedNode), Without<GraphNode>>,
    nodes: Query<(Entity, &NodePosition, &ComputedNode, &GraphOf), With<NodeKey>>,
    mut minis: Query<(Entity, &MiniNode, &mut Node, &ChildOf), Without<MiniView>>,
    mut view_rect: Query<(&mut Node, &ChildOf), With<MiniView>>,
) {
    let Some(ec) = ec else { return };
    let mut minimap = minimap;
    let Ok((map_e, mut vis)) = minimap.single_mut() else { return };
    // `G`'s nodes and this minimap's own rectangles only.
    let own = |g: &GraphOf| g.0 == G::VIEW;
    let mapping = Mapping::new(nodes.iter().filter(|n| own(n.3)).map(|(_, p, c, _)| node_rect(p, c)));
    let want = if mapping.is_some() { Visibility::Inherited } else { Visibility::Hidden };
    vis.set_if_neq(want);
    let (Some(map), Ok((view, canvas))) = (mapping, views.get(ec.canvas)) else { return };
    let place = |node: &mut Node, rect: Rect| {
        let r = Rect::from_corners(map.to_map(rect.min), map.to_map(rect.max));
        (node.left, node.top) = (px(r.min.x), px(r.min.y));
        (node.width, node.height) = (px(r.width().max(1.0)), px(r.height().max(1.0)));
    };
    for (e, mini, mut node, _) in minis.iter_mut().filter(|m| m.3.parent() == map_e) {
        match nodes.get(mini.0).ok().filter(|n| own(n.3)) {
            Some((_, p, c, _)) => place(&mut node, node_rect(p, c)),
            None => commands.entity(e).try_despawn(),
        }
    }
    let shown = |n: Entity| minis.iter().any(|m| m.3.parent() == map_e && m.1.0 == n);
    for (n, ..) in nodes.iter().filter(|(n, .., g)| own(g) && !shown(*n)) {
        commands.spawn((
            MiniNode(n),
            Node { position_type: PositionType::Absolute, ..default() },
            BackgroundColor(Color::srgba(0.4, 0.6, 0.95, 0.6)),
            Pickable::IGNORE,
            ChildOf(map_e),
        ));
    }
    if let Some((mut vr, _)) = view_rect.iter_mut().find(|v| v.1.parent() == map_e) {
        let size = canvas.size() * canvas.inverse_scale_factor();
        place(&mut vr, Rect::from_corners(view.canvas_to_graph(Vec2::ZERO), view.canvas_to_graph(size)));
    }
}

fn look_here<G: GraphDomain, E: EntityEvent + bevy::picking::events::PointerEvent>(
    ev: On<E>,
    ec: Option<Res<EditorCanvas<G>>>,
    maps: Query<(&ComputedNode, &UiGlobalTransform), With<Minimap<G>>>,
    mut views: Query<(&mut CanvasView, &ComputedNode), Without<Minimap<G>>>,
    nodes: Query<(&NodePosition, &ComputedNode, &GraphOf), With<NodeKey>>,
) {
    let Some(ec) = ec else { return };
    let rects = nodes.iter().filter(|n| n.2.0 == G::VIEW).map(|(p, c, _)| node_rect(p, c));
    let (Ok((map, t)), Some(mapping)) = (maps.get(ev.event_target()), Mapping::new(rects)) else {
        return;
    };
    let Some(n) = map.normalize_point(*t, ev.pointer().position / map.inverse_scale_factor()) else { return };
    let target = mapping.to_graph((n + 0.5) * MAP);
    if let Ok((mut view, canvas)) = views.get_mut(ec.canvas) {
        let size = canvas.size() * canvas.inverse_scale_factor();
        view.pan = size / 2.0 - target * view.zoom;
    }
}
