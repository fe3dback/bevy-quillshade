//! bevy_noodle canvas of the active document.
//!
//! One-way sync (spec §14): the model is the source of truth and the canvas is rebuilt from it;
//! user gestures arrive as `EditApplied` (origin `Interaction`) and become [`Action`]s.

use std::collections::{BTreeSet, HashMap, HashSet};

use bevy::prelude::*;
use bevy::ui::Selected;
use bevy::ui::ui_transform::UiGlobalTransform;
use bevy_noodle::prelude::*;
use bevy_noodle::style::kit;
use shader_graph_core::model::{Asset, NodeId};
use shader_graph_core::registry::{FUNCTION_PREFIX, NodeRole};
use shader_graph_core::{Code, PinType, Value};
use shader_project::AssetId;

use crate::doc::{connection_error, node_signature};
use crate::state::{Action, Actions, Editor, Link, ViewRequest};

/// Moves a frame makes to carry the nodes inside it.
pub const CARRIED: EditOrigin = EditOrigin::Custom(1);

/// Model id on a node entity.
#[derive(Component, Default, Clone, Debug)]
pub struct NodeKey(pub String);

/// Comment frame id on a frame entity.
#[derive(Component, Default, Clone, Debug)]
pub struct FrameKey(pub String);

/// Model pin on a port entity.
#[derive(Component, Default, Clone, Debug, PartialEq)]
pub struct PinKey {
    pub node: String,
    pub pin: String,
    pub output: bool,
}

/// Label of a pin row (recolored by diagnostics).
#[derive(Component, Default, Clone, Debug)]
pub struct PinLabel {
    pub node: String,
    pub pin: String,
    pub base: Color,
}

#[derive(Resource)]
pub struct EditorCanvas {
    pub canvas: Entity,
    /// Document and revision currently on the canvas.
    pub shown: Option<(AssetId, u64)>,
    /// Saved pan/zoom per document.
    views: HashMap<AssetId, (Vec2, f32)>,
    /// True while the canvas replays the model (connection checks are skipped).
    pub syncing: bool,
}

impl EditorCanvas {
    pub fn new(canvas: Entity) -> Self {
        EditorCanvas { canvas, shown: None, views: HashMap::new(), syncing: false }
    }
}

/// Moved entities waiting to be committed as one undo step.
#[derive(Resource, Default)]
pub struct PendingMoves {
    entities: HashSet<Entity>,
    quiet_frames: u8,
}

pub struct CanvasPlugin;

impl Plugin for CanvasPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PendingMoves>()
            .add_systems(
                Update,
                (
                    sync_from_canvas,
                    commit_moves,
                    sync_selection,
                    rebuild_canvas,
                    highlight_diagnostics,
                    keep_center_on_resize,
                    apply_view_requests,
                )
                    .chain()
                    .in_set(CanvasSystems),
            )
            .add_observer(check_connection)
            .add_observer(carry_frame_contents);
    }
}

#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct CanvasSystems;

// ---------------------------------------------------------------------------------------------
// Look

pub fn pin_color(t: PinType) -> Color {
    let [r, g, b] = t.color();
    Color::srgb(r, g, b)
}

fn category_color(cat: &str) -> Color {
    match cat {
        "Constants" => Color::srgb_u8(70, 98, 58),
        "Parameters" => Color::srgb_u8(48, 104, 82),
        "Math" => Color::srgb_u8(66, 84, 60),
        "Vector" => Color::srgb_u8(82, 78, 52),
        "Coordinates" => Color::srgb_u8(110, 58, 58),
        "Texture" => Color::srgb_u8(50, 74, 120),
        "Utility" => Color::srgb_u8(74, 74, 80),
        "Output" => Color::srgb_u8(90, 64, 34),
        "Function" | "Functions" => Color::srgb_u8(86, 60, 110),
        _ => Color::srgb_u8(60, 63, 71),
    }
}

const ERROR: Color = Color::srgb(0.95, 0.30, 0.30);
const WARNING: Color = Color::srgb(0.95, 0.75, 0.25);
const LABEL: Color = Color::srgb(0.84, 0.85, 0.88);
const LABEL_DIM: Color = Color::srgb(0.45, 0.46, 0.50);

fn short(v: &Value) -> String {
    let f = |x: f32| {
        let s = format!("{x:.3}");
        let s = s.trim_end_matches('0').trim_end_matches('.').to_string();
        if s.is_empty() || s == "-" { "0".into() } else { s }
    };
    match v {
        Value::Float(x) => f(*x),
        Value::Int(i) => i.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Str(s) => s.clone(),
        Value::Type(t) => t.label().to_string(),
        other => format!("({})", other.floats().iter().map(|x| f(*x)).collect::<Vec<_>>().join(", ")),
    }
}

/// Title shown on a node: kind name, or the parameter/function/constant it stands for.
pub fn node_title(editor: &Editor, asset: &Asset, id: &str) -> String {
    let Some(n) = asset.nodes.get(id) else { return String::new() };
    let Some(def) = editor.registry.get(&n.kind) else { return format!("? {}", n.kind) };
    let params = editor.registry.resolved_params(&n.kind, &n.params);
    let name = def.info().name.clone();
    let p = |k: &str| params.get(k).map(short).unwrap_or_default();
    match def.role() {
        NodeRole::Parameter => format!("{} ({name})", p("name")),
        NodeRole::FunctionInput => format!("Input: {}", p("name")),
        NodeRole::FunctionOutput => format!("Output: {}", p("name")),
        _ if n.kind.starts_with(FUNCTION_PREFIX) => {
            let path = &n.kind[FUNCTION_PREFIX.len()..];
            format!("fn {}", editor.functions.get(path).map_or(path, |s| s.name.as_str()))
        }
        _ if n.kind.starts_with("stdlib::Constant") => params.get("value").map(short).unwrap_or(name),
        _ => name,
    }
}

fn pin_row(label: String, port: Port, color: Color, key: PinKey, text: Color) -> impl Scene {
    let justify = match port.direction {
        PortDirection::Input => JustifyContent::FlexStart,
        PortDirection::Output => JustifyContent::FlexEnd,
    };
    let tag = PinLabel { node: key.node.clone(), pin: key.pin.clone(), base: text };
    bsn! {
        Node {
            justify_content: justify,
            align_items: AlignItems::Center,
            min_height: px(22),
            margin: UiRect::horizontal(px(-10)),
            padding: UiRect::horizontal(px(10)),
        }
        Children [
            Text(label)
            TextFont { font_size: FontSize::Px(13.0) }
            TextColor(text)
            tag
            Pickable { should_block_lower: false, is_hoverable: false }
            --
            @kit::port(port, color) key
        ]
    }
}

/// Scene of one model node.
fn node_scene(editor: &Editor, asset: &Asset, path: &str, id: &str, canvas: Entity) -> Option<Box<dyn Scene>> {
    let n = asset.nodes.get(id)?;
    let pos = Vec2::new(n.pos.0, n.pos.1);
    let key = NodeKey(id.to_string());
    let def = editor.registry.get(&n.kind);
    let sig = node_signature(asset, id, &editor.registry, &editor.functions, path);
    if def.is_some_and(|d| d.role() == NodeRole::Reroute) {
        let ty = sig.as_ref().and_then(|s| s.outputs.first()).map_or(PinType::Any, |p| p.ty);
        let c = pin_color(ty);
        let kin = PinKey { node: id.into(), pin: "in".into(), output: false };
        let kout = PinKey { node: id.into(), pin: "out".into(), output: true };
        return Some(Box::new(bsn! {
            GraphNode
            NodePosition(pos)
            key
            Node { width: px(20), height: px(20), border: UiRect::all(px(1.5)), border_radius: BorderRadius::MAX }
            BackgroundColor(Color::srgb_u8(44, 46, 52))
            SelectedBorderColor
            ChildOf(canvas)
            Children [
                @kit::port(Port::input(PortType::ANY), c) kin
                --
                @kit::port(Port::output(PortType::ANY), c) kout
            ]
        }));
    }
    let title = node_title(editor, asset, id);
    let cat = def.map_or("Unknown".to_string(), |d| d.info().category.clone());
    let head = category_color(&cat);
    let mut rows: Vec<Box<dyn Scene>> = Vec::new();
    if let Some(sig) = &sig {
        for p in &sig.outputs {
            let key = PinKey { node: id.into(), pin: p.id.clone(), output: true };
            let label = if p.label.is_empty() { p.ty.label().to_string() } else { p.label.clone() };
            rows.push(Box::new(pin_row(label, Port::output(PortType::ANY), pin_color(p.ty), key, LABEL)));
        }
        for p in &sig.inputs {
            let key = PinKey { node: id.into(), pin: p.id.clone(), output: false };
            let connected = asset.edge_into(id, &p.id).is_some();
            let mut label = if p.label.is_empty() { p.id.clone() } else { p.label.clone() };
            if !connected {
                if let Some(what) = &p.implicit {
                    label.push_str(&format!(" ({what})"));
                } else if p.ty.is_numeric() && !p.required {
                    let v = n.inputs.get(&p.id).cloned().unwrap_or_else(|| p.default.coerce(p.ty));
                    label.push_str(&format!(" = {}", short(&v)));
                }
            }
            let text = if p.inactive { LABEL_DIM } else { LABEL };
            rows.push(Box::new(pin_row(label, Port::input(PortType::ANY), pin_color(p.ty), key, text)));
        }
    } else {
        rows.push(Box::new(
            bsn! { Text("unknown node kind") TextColor(ERROR) TextFont { font_size: FontSize::Px(12.0) } },
        ));
    }
    Some(Box::new(bsn! {
        @kit::node(pos)
        Node { min_width: px(150) }
        key
        ChildOf(canvas)
        Children [ @kit::title(title) BackgroundColor(head) -- {rows} ]
    }))
}

fn frame_scene(f: &shader_graph_core::Frame, canvas: Entity) -> impl Scene {
    let [r, g, b, _] = f.color.unwrap_or([0.55, 0.62, 0.75, 1.0]);
    let c = Color::srgb(r, g, b);
    let (x, y, w, h) = f.rect;
    let (pos, title, key) = (Vec2::new(x, y), f.title.clone(), FrameKey(f.id.clone()));
    let border = SelectedBorderColor { normal: c.with_alpha(0.5), selected: Color::WHITE };
    let bg = BackgroundColor(c.with_alpha(0.08));
    bsn! {
        GraphNode
        NodePosition(pos)
        key
        ZIndex(-1)
        Node {
            width: px(w),
            height: px(h),
            padding: UiRect::all(px(8)),
            border: UiRect::all(px(1.5)),
            border_radius: BorderRadius::all(px(10)),
            align_items: AlignItems::FlexStart,
        }
        bg
        border
        ChildOf(canvas)
        Children [
            Text(title)
            TextFont { font_size: FontSize::Px(16.0) }
            TextColor(c)
            Pickable { should_block_lower: false, is_hoverable: false }
        ]
    }
}

// ---------------------------------------------------------------------------------------------
// Model → canvas

fn idle(world: &mut World) -> bool {
    let wires = world.query::<&PendingWire>().iter(world).next().is_some();
    let pressed = world
        .get_resource::<ButtonInput<MouseButton>>()
        .is_some_and(|b| b.any_pressed([MouseButton::Left, MouseButton::Right, MouseButton::Middle]));
    !wires && !pressed
}

/// Rebuilds the canvas when the active document or its revision changed (and no gesture is running).
pub fn rebuild_canvas(world: &mut World) {
    let Some(ec) = world.get_resource::<EditorCanvas>() else { return };
    let (canvas, shown) = (ec.canvas, ec.shown.clone());
    let editor = world.resource::<Editor>();
    let want = editor.doc().map(|d| (d.id.clone(), d.revision));
    if want == shown {
        return;
    }
    let same_doc = want.as_ref().map(|w| &w.0) == shown.as_ref().map(|s| &s.0);
    if same_doc && !idle(world) {
        return;
    }

    // Collect everything from the model first (observers below read `Editor`).
    let editor = world.resource::<Editor>();
    let mut scenes: Vec<Box<dyn Scene>> = Vec::new();
    let mut edges = Vec::new();
    let (mut selection, mut frame_sel) = (BTreeSet::new(), None);
    if let Some(d) = editor.doc() {
        for f in &d.asset.comments {
            scenes.push(Box::new(frame_scene(f, canvas)));
        }
        for id in d.asset.nodes.keys() {
            if let Some(s) = node_scene(editor, &d.asset, &d.id.path, id, canvas) {
                scenes.push(s);
            }
        }
        edges = d.asset.edges.clone();
        selection = d.selection.clone();
        frame_sel = d.selected_frame.clone();
    }

    // View per document.
    if !same_doc {
        let view = world.get::<CanvasView>(canvas).map(|v| (v.pan, v.zoom));
        let mut ec = world.resource_mut::<EditorCanvas>();
        if let (Some((old, _)), Some(v)) = (&shown, view) {
            ec.views.insert(old.clone(), v);
        }
        let restore = want.as_ref().and_then(|(id, _)| ec.views.get(id).copied());
        if let Some((pan, zoom)) = restore
            && let Some(mut v) = world.get_mut::<CanvasView>(canvas)
        {
            v.pan = pan;
            v.zoom = zoom;
        }
    }

    let old: Vec<Entity> = world.query_filtered::<Entity, Or<(With<NodeKey>, With<FrameKey>)>>().iter(world).collect();
    for e in old {
        world.entity_mut(e).despawn();
    }
    for s in scenes {
        if let Err(e) = world.spawn_scene(s) {
            error!("spawning a node failed: {e}");
        }
    }
    world.flush();

    let mut ports: HashMap<(String, String, bool), Entity> = HashMap::new();
    for (e, k) in world.query::<(Entity, &PinKey)>().iter(world) {
        ports.insert((k.node.clone(), k.pin.clone(), k.output), e);
    }
    world.resource_mut::<EditorCanvas>().syncing = true;
    for edge in &edges {
        let from = ports.get(&(edge.from.0.clone(), edge.from.1.clone(), true));
        let to = ports.get(&(edge.to.0.clone(), edge.to.1.clone(), false));
        if let (Some(&from), Some(&to)) = (from, to)
            && let Err(e) = world.graph_edit(canvas, GraphEdit::Connect { from, to })
        {
            warn!("cannot show edge {:?} -> {:?}: {e:?}", edge.from, edge.to);
        }
    }
    world.resource_mut::<EditorCanvas>().syncing = false;

    let mut picked = Vec::new();
    for (e, k) in world.query::<(Entity, &NodeKey)>().iter(world) {
        if selection.contains(&k.0) {
            picked.push(e);
        }
    }
    for (e, k) in world.query::<(Entity, &FrameKey)>().iter(world) {
        if frame_sel.as_ref() == Some(&k.0) {
            picked.push(e);
        }
    }
    world.select(canvas, picked, SelectMode::Replace);
    world.resource_mut::<EditorCanvas>().shown = want;
}

/// Error/warning outlines on nodes and colored pin labels.
fn highlight_diagnostics(
    editor: Res<Editor>,
    nodes: Query<(Entity, &NodeKey, Option<&Outline>)>,
    mut labels: Query<(&PinLabel, &mut TextColor)>,
    mut commands: Commands,
) {
    let diags = editor.node_diags();
    for (e, key, outline) in &nodes {
        let level = diags.get(&key.0).map(|v| v.iter().any(|d| d.is_error()));
        let color = match level {
            Some(true) => Some(ERROR),
            Some(false) => Some(WARNING),
            None => None,
        };
        match (color, outline) {
            (Some(c), o) if o.is_none_or(|o| o.color != c) => {
                commands.entity(e).try_insert(Outline::new(px(2), px(2), c));
            }
            (None, Some(_)) => {
                commands.entity(e).try_remove::<Outline>();
            }
            _ => {}
        }
    }
    for (label, mut color) in &mut labels {
        let pin_diag = diags.get(&label.node).and_then(|v| {
            v.iter()
                .filter(|d| d.pin.as_deref() == Some(label.pin.as_str()) && d.code != Code::W102)
                .map(|d| d.is_error())
                .reduce(|a, b| a || b)
        });
        let want = match pin_diag {
            Some(true) => ERROR,
            Some(false) => WARNING,
            None => label.base,
        };
        color.set_if_neq(TextColor(want));
    }
}

// ---------------------------------------------------------------------------------------------
// Canvas → model

/// Validates a wire against the model rules (types 7.2, cycles); runs for candidates and drops.
fn check_connection(
    mut check: On<ConnectionCheck>,
    keys: Query<&PinKey>,
    editor: Option<Res<Editor>>,
    ec: Option<Res<EditorCanvas>>,
) {
    if ec.as_ref().is_none_or(|c| c.syncing || c.canvas != check.canvas) {
        return;
    }
    let (Ok(a), Ok(b), Some(editor)) = (keys.get(check.ports.output), keys.get(check.ports.input), editor) else {
        return;
    };
    let Some(d) = editor.doc() else { return };
    if connection_error(&d.asset, (&a.node, &a.pin), (&b.node, &b.pin), &editor.registry, &editor.functions, &d.id.path)
        .is_some()
    {
        check.reject();
    }
}

fn sync_from_canvas(
    mut applied: MessageReader<EditApplied>,
    keys: Query<&PinKey>,
    ec: Option<Res<EditorCanvas>>,
    mut moves: ResMut<PendingMoves>,
    mut actions: ResMut<Actions>,
) {
    let Some(ec) = ec else { return };
    let mut connected_inputs = Vec::new();
    let mut out = Vec::new();
    for msg in applied.read() {
        if msg.canvas != ec.canvas || !(msg.origin == EditOrigin::Interaction || msg.origin == CARRIED) {
            continue;
        }
        match &msg.change {
            GraphChange::Connected { ports, .. } => {
                if let (Ok(a), Ok(b)) = (keys.get(ports.output), keys.get(ports.input)) {
                    connected_inputs.push((b.node.clone(), b.pin.clone()));
                    out.push(Action::Connect {
                        from: (a.node.clone(), a.pin.clone()),
                        to: (b.node.clone(), b.pin.clone()),
                    });
                }
            }
            GraphChange::Disconnected { ports, .. } => {
                if let Ok(b) = keys.get(ports.input) {
                    out.push(Action::Disconnect { to: (b.node.clone(), b.pin.clone()) });
                }
            }
            GraphChange::Moved { nodes, drag, .. } => {
                if drag.is_none_or(|d| d.is_final) {
                    moves.entities.extend(nodes.iter().copied());
                    moves.quiet_frames = 0;
                }
            }
            GraphChange::Deleted { .. } => {}
        }
    }
    // A connect into a used input reports the replaced edge first; the model connect replaces it anyway.
    out.retain(|a| !matches!(a, Action::Disconnect { to } if connected_inputs.contains(to)));
    actions.0.extend(out);
}

/// Commits finished moves (including nodes carried by frames) as one action, a frame later.
fn commit_moves(
    mut moves: ResMut<PendingMoves>,
    nodes: Query<(&NodeKey, &NodePosition)>,
    frames: Query<(&FrameKey, &NodePosition)>,
    mut actions: ResMut<Actions>,
) {
    if moves.entities.is_empty() {
        return;
    }
    moves.quiet_frames += 1;
    if moves.quiet_frames < 2 {
        return;
    }
    let ents = std::mem::take(&mut moves.entities);
    let mut n = Vec::new();
    let mut f = Vec::new();
    for e in ents {
        if let Ok((k, p)) = nodes.get(e) {
            n.push((k.0.clone(), (p.0.x, p.0.y)));
        } else if let Ok((k, p)) = frames.get(e) {
            f.push((k.0.clone(), (p.0.x, p.0.y)));
        }
    }
    n.sort_by(|a, b| a.0.cmp(&b.0));
    actions.push(Action::Moved { nodes: n, frames: f });
}

/// When a frame is dragged, the nodes inside it move along (same drag, so one gesture).
fn carry_frame_contents(
    applied: On<EditApplied>,
    frames: Query<(&NodePosition, &ComputedNode), With<FrameKey>>,
    nodes: Query<(Entity, &NodePosition, &ComputedNode), With<NodeKey>>,
    mut commands: Commands,
) {
    let GraphChange::Moved { nodes: moved, delta, drag } = &applied.change else { return };
    if applied.origin == CARRIED {
        return;
    }
    let rect = |p: Vec2, c: &ComputedNode| Rect::from_corners(p, p + c.size() * c.inverse_scale_factor());
    let mut carried = Vec::new();
    for (frame, computed) in moved.iter().filter_map(|f| frames.get(*f).ok()) {
        let before = rect(frame.0 - *delta, computed);
        for (node, p, c) in &nodes {
            let r = rect(p.0, c);
            if before.contains(r.min) && before.contains(r.max) && !moved.contains(&node) && !carried.contains(&node) {
                carried.push(node);
            }
        }
    }
    if !carried.is_empty() {
        let edit = GraphEdit::MoveNodes { nodes: carried, delta: *delta, drag: *drag };
        commands.graph_edit_with_origin(applied.canvas, edit, CARRIED);
    }
}

fn sync_selection(
    ec: Option<Res<EditorCanvas>>,
    editor: Res<Editor>,
    selected: Query<(Option<&NodeKey>, Option<&FrameKey>), With<Selected>>,
    mut actions: ResMut<Actions>,
) {
    let Some(ec) = ec else { return };
    let Some(d) = editor.doc() else { return };
    if ec.shown.as_ref().map(|s| &s.0) != Some(&d.id) {
        return;
    }
    let mut nodes = BTreeSet::new();
    let mut frame = None;
    for (n, f) in &selected {
        if let Some(n) = n {
            nodes.insert(n.0.clone());
        }
        if let Some(f) = f {
            frame = Some(f.0.clone());
        }
    }
    if nodes != d.selection {
        actions.push(Action::Select(nodes.clone()));
    }
    if nodes.is_empty() && frame != d.selected_frame {
        actions.push(Action::SelectFrame(frame));
    }
}

// ---------------------------------------------------------------------------------------------
// View

/// Converts a window position (logical px) into graph space of `canvas`.
pub fn window_to_graph(
    world_pos: Vec2,
    computed: &ComputedNode,
    transform: &UiGlobalTransform,
    view: &CanvasView,
) -> Option<Vec2> {
    let n = computed.normalize_point(*transform, world_pos / computed.inverse_scale_factor())?;
    let local = (n + 0.5) * computed.size() * computed.inverse_scale_factor();
    Some(view.canvas_to_graph(local))
}

fn apply_view_requests(
    mut editor: ResMut<Editor>,
    ec: Option<Res<EditorCanvas>>,
    mut views: Query<(&mut CanvasView, &ComputedNode)>,
    nodes: Query<(&NodePosition, &ComputedNode, &NodeKey)>,
) {
    let Some(ec) = ec else { return };
    if editor.view_requests.is_empty() {
        return;
    }
    let Some(d) = editor.doc() else {
        editor.view_requests.clear();
        return;
    };
    if ec.shown.as_ref().map(|s| &s.0) != Some(&d.id) {
        return;
    }
    let Ok((mut view, canvas)) = views.get_mut(ec.canvas) else { return };
    let size = canvas.size() * canvas.inverse_scale_factor();
    if size.min_element() <= 1.0 {
        return;
    }
    let req = editor.view_requests[0].clone();
    let sel = &d.selection;
    let rects: Vec<Rect> = nodes
        .iter()
        .filter(|(_, _, k)| req == ViewRequest::FitAll || sel.is_empty() || sel.contains(&k.0))
        .map(|(p, c, _)| Rect::from_corners(p.0, p.0 + c.size() * c.inverse_scale_factor()))
        .collect();
    if rects.iter().any(|r| r.size().min_element() <= 0.0) {
        return; // not laid out yet
    }
    editor.view_requests.remove(0);
    let Some(bounds) = rects.into_iter().reduce(|a, b| a.union(b)) else { return };
    let bounds = bounds.inflate(60.0);
    let fit = size / bounds.size();
    view.zoom = fit.min_element().clamp(view.min_zoom.max(0.1), 1.0);
    view.pan = size / 2.0 - bounds.center() * view.zoom;
}

/// Resizing the window keeps the graph point at the canvas center in place.
fn keep_center_on_resize(
    ec: Option<Res<EditorCanvas>>,
    mut views: Query<(&mut CanvasView, &ComputedNode)>,
    mut last: Local<Vec2>,
) {
    let Some(ec) = ec else { return };
    let Ok((mut view, c)) = views.get_mut(ec.canvas) else { return };
    let size = c.size() * c.inverse_scale_factor();
    if *last != Vec2::ZERO && size != *last {
        view.pan += (size - *last) / 2.0;
    }
    *last = size;
}

/// Graph-space center of the visible canvas.
pub fn view_center(world: &World, canvas: Entity) -> Vec2 {
    let (Some(view), Some(c)) = (world.get::<CanvasView>(canvas), world.get::<ComputedNode>(canvas)) else {
        return Vec2::ZERO;
    };
    view.canvas_to_graph(c.size() * c.inverse_scale_factor() / 2.0)
}

/// Link for a palette opened from a wire drop.
pub fn link_of(key: &PinKey) -> Link {
    Link { node: key.node.clone(), pin: key.pin.clone(), from_output: key.output }
}

pub fn node_ids(world: &mut World) -> Vec<(Entity, NodeId)> {
    world.query::<(Entity, &NodeKey)>().iter(world).map(|(e, k)| (e, k.0.clone())).collect()
}
