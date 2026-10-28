//! bevy_noodle canvas of the active document.
//!
//! One-way sync (spec §14): the model is the source of truth and the canvas is rebuilt from it;
//! user gestures arrive as `EditApplied` (origin `Interaction`) and become [`Action`]s.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use bevy::feathers::controls::NumberInputValue;
use bevy::input_focus::InputFocus;
use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::text::{EditableText, TextReadWriteMode};
use bevy::ui::Selected;
use bevy::ui::ui_transform::UiGlobalTransform;
use bevy::ui_widgets::popover::Popover;
use bevy_noodle::prelude::*;
use bevy_noodle::style::kit;
use shader_graph_core::model::{Asset, AssetKind, Frame, NodeId};
use shader_graph_core::registry::{FUNCTION_PREFIX, NodeRole, ParamKind, ROUTE_PARAM, Registry};
use shader_graph_core::{Code, PinType, Value};
use shader_project::AssetId;

use crate::doc::{connection_error, node_signature, route_declarations, tidy};
use crate::editors::main_input::typing_into;
use crate::inline::{Inline, inline_for_param, inline_for_pin, inline_scene};
use crate::state::{Action, Actions, Editor, Link, PropTarget, ViewRequest, display_value};
use crate::ui::icons::{IconId, icon};
use crate::ui::props::PropField;
use crate::ui::{ACCENT, node_preview};

/// Model id on a node entity.
#[derive(Component, Default, Clone, Debug)]
pub struct NodeKey(pub String);

/// Comment frame id on a frame entity.
#[derive(Component, Default, Clone, Debug)]
pub struct FrameKey(pub String);

/// On a node's title bar: a double click there renames the node (design §5.5).
#[derive(Component, Default, Clone, Debug)]
pub struct NodeHeader;

/// On a comment frame's title (a strip across the frame's top): a double click there edits the title (design §5.5).
#[derive(Component, Default, Clone, Debug)]
pub struct FrameTitleStrip;

/// Model pin on a port entity.
#[derive(Component, Default, Clone, Debug, PartialEq)]
pub struct PinKey {
    pub node: String,
    pub pin: String,
    pub output: bool,
}

/// On the previewed node (decision 0022): its outline pulses (`node_outlines`).
#[derive(Component, Default, Clone, Debug)]
pub struct PreviewPulse;

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
            .init_resource::<MoveVerdicts>()
            .add_systems(
                Update,
                (
                    sync_from_canvas,
                    commit_moves,
                    sync_selection,
                    rebuild_canvas,
                    frame_colors,
                    sync_inline_fields,
                    highlight_diagnostics,
                    node_outlines,
                    keep_center_on_resize,
                    apply_view_requests,
                )
                    .chain()
                    .in_set(CanvasSystems),
            )
            .add_observer(check_connection)
            .add_observer(start_move_verdicts)
            .add_observer(check_move)
            .add_observer(request_move_wires);
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
        "Color" => Color::srgb_u8(104, 62, 96),
        "Output" => Color::srgb_u8(90, 64, 34),
        "Function" | "Functions" => Color::srgb_u8(86, 60, 110),
        _ => Color::srgb_u8(60, 63, 71),
    }
}

/// Hue in degrees from a stable hash of a name (FNV-1a 32-bit of its UTF-8 bytes), design §4.2.
pub(crate) fn name_hue(name: &str) -> f32 {
    let hash = name.bytes().fold(2166136261u32, |h, b| (h ^ b as u32).wrapping_mul(16777619));
    (hash % 360) as f32
}

/// Header colour of a node named by the user: fixed saturation / lightness, hue from the name (design §4.2).
pub(crate) fn name_header_color(name: &str) -> Color {
    Color::hsl(name_hue(name), 0.45, 0.32)
}

/// The declared name of the route a Named Route Usage reads (its resolved `params`), looked up in `routes`
/// ([`route_declarations`] of its asset); `None` when the declaration is missing.
fn usage_route<'a>(routes: &'a [(NodeId, String)], params: &BTreeMap<String, Value>) -> Option<&'a str> {
    let route = params.get(ROUTE_PARAM).map_or("", Value::as_str);
    routes.iter().find(|r| r.0 == route).map(|r| r.1.as_str())
}

/// Name a node's header colour comes from (design §4.2): a named route's name (usages: their declaration's, from
/// `routes`), a constant's user title, a `Global*` node's constant. `None`: the category colour.
fn header_name(registry: &Registry, asset: &Asset, id: &str, routes: &[(NodeId, String)]) -> Option<String> {
    let n = asset.nodes.get(id)?;
    let def = registry.get(&n.kind)?;
    let params = registry.resolved_params(&n.kind, &n.params);
    let name = match def.role() {
        NodeRole::RouteDeclaration => params.get("name").map(|v| v.as_str().to_string()),
        NodeRole::RouteUsage => usage_route(routes, &params).map(str::to_string),
        _ if has_label(&n.kind) => Some(n.label.clone()),
        _ => def
            .params()
            .into_iter()
            .find(|p| matches!(p.kind, ParamKind::GlobalConst(_)))
            .and_then(|p| params.get(&p.id).map(|v| v.as_str().to_string())),
    };
    name.filter(|s| !s.is_empty())
}

/// Header colour of a node: by name (see [`header_name`]) or by category.
pub(crate) fn header_color(registry: &Registry, asset: &Asset, id: &str, routes: &[(NodeId, String)]) -> Color {
    if let Some(name) = header_name(registry, asset, id, routes) {
        return name_header_color(&name);
    }
    let cat = asset.nodes.get(id).and_then(|n| registry.get(&n.kind)).map(|d| d.info().category.clone());
    category_color(cat.as_deref().unwrap_or("Unknown"))
}

/// Selected node outline: white like a selected comment frame's border (design §2), so it never reads as [`WARNING`].
const SELECTED: Color = Color::WHITE;
/// Period of the previewed node's outline pulse, seconds (design §4.4).
const PULSE_PERIOD: f32 = 1.2;

/// Previewed node outline colour at `t` seconds: a smooth sine between bright orange (t = 0) and dark red.
pub(crate) fn pulse_color(t: f32) -> Color {
    let k = 0.5 - 0.5 * (t * std::f32::consts::TAU / PULSE_PERIOD).cos();
    let (a, b) = (Color::srgb_u8(255, 140, 0).to_srgba(), Color::srgb_u8(140, 20, 20).to_srgba());
    Color::Srgba(a.mix(&b, k))
}

/// Emphasis outline of a node (design §4.3, §4.4): previewed 6 px pulsing; selected 3 px (diagnostic colour if
/// any); diagnostics alone 2 px. `diag`: worst diagnostic, `Some(true)` for an error. The node border stays 1.5 px:
/// an outline changes no layout, so ports and wires don't move.
pub(crate) fn node_outline(previewed: bool, selected: bool, diag: Option<bool>, t: f32) -> Option<Outline> {
    let diag = diag.map(|error| if error { ERROR } else { WARNING });
    if previewed {
        Some(Outline::new(px(6), px(0), pulse_color(t)))
    } else if selected {
        Some(Outline::new(px(3), px(0), diag.unwrap_or(SELECTED)))
    } else {
        diag.map(|c| Outline::new(px(2), px(2), c))
    }
}

/// A hovered pin's dot is shown at least this many times its size (design §4.8) …
pub const PIN_HOVER_SCALE: f32 = 1.3;
/// … and this much lighter (bevy_noodle's `PortHoverStyle`, set on the canvas in `attach_canvas`).
pub const PIN_HOVER_LIGHTEN: f32 = 0.15;

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
/// Constant nodes carry an editor-only label that replaces the value in the title (decision 0021).
pub fn has_label(kind: &str) -> bool {
    matches!(
        kind,
        "stdlib::Constant" | "stdlib::Constant2Vector" | "stdlib::Constant3Vector" | "stdlib::Constant4Vector"
    )
}

pub fn node_title(editor: &Editor, asset: &Asset, id: &str) -> String {
    node_title_in(editor, asset, id, &route_declarations(asset, &editor.registry))
}

/// [`node_title`] with the asset's [`route_declarations`] already at hand (a canvas rebuild computes them once).
fn node_title_in(editor: &Editor, asset: &Asset, id: &str, routes: &[(NodeId, String)]) -> String {
    let Some(n) = asset.nodes.get(id) else { return String::new() };
    let Some(def) = editor.registry.get(&n.kind) else { return format!("? {}", n.kind) };
    let params = editor.registry.resolved_params(&n.kind, &n.params);
    let name = def.info().name.clone();
    let p = |k: &str| params.get(k).map(short).unwrap_or_default();
    match def.role() {
        NodeRole::Parameter | NodeRole::RouteDeclaration => format!("{} ({name})", p("name")),
        NodeRole::RouteUsage => format!("{} ({name})", usage_route(routes, &params).unwrap_or("?")),
        NodeRole::FunctionInput => format!("Input: {}", p("name")),
        NodeRole::FunctionOutput => format!("Output: {}", p("name")),
        _ if n.kind.starts_with(FUNCTION_PREFIX) => {
            let path = &n.kind[FUNCTION_PREFIX.len()..];
            format!("fn {}", editor.functions.get(path).map_or(path, |s| s.name.as_str()))
        }
        _ if has_label(&n.kind) && !n.label.is_empty() => n.label.clone(),
        _ if n.kind.starts_with("stdlib::Constant") => params.get("value").map(short).unwrap_or(name),
        _ => {
            // Sample Curve: the curve's name (decision 0015); Global*: the constant's name (decision 0017); texture
            // nodes: the Texture asset's name; Sample Gradient: the collection's name (the gradient's name shows in
            // its properties: the canvas rebuilds only on graph revisions, not on edits of the collection).
            let curve = def
                .params()
                .into_iter()
                .find(|p| {
                    matches!(
                        p.kind,
                        ParamKind::Curve
                            | ParamKind::GlobalConst(_)
                            | ParamKind::Texture
                            | ParamKind::GradientCollection
                    )
                })
                .map(|c| p(&c.id));
            match curve.filter(|c| !c.is_empty()) {
                Some(path) => format!("{} ({name})", path.rsplit('/').next().unwrap_or(&path)),
                None => name,
            }
        }
    }
}

fn pin_row(
    label: String,
    port: Port,
    color: Color,
    key: PinKey,
    text: Color,
    editor: Option<Box<dyn Scene>>,
) -> impl Scene {
    let justify = match port.direction {
        PortDirection::Input => JustifyContent::FlexStart,
        PortDirection::Output => JustifyContent::FlexEnd,
    };
    let tag = PinLabel { node: key.node.clone(), pin: key.pin.clone(), base: text };
    // Only rows with an editor get the growing slot: an empty one would push labels off the right edge.
    let editor: Vec<Box<dyn Scene>> = editor
        .map(|e| -> Box<dyn Scene> {
            Box::new(bsn! {
                Node { flex_grow: 1.0, margin: UiRect::left(px(6)), align_items: AlignItems::Center } Children [ @{e} ]
            })
        })
        .into_iter()
        .collect();
    bsn! {
        Node {
            justify_content: justify,
            align_items: AlignItems::Center,
            min_height: px(PIN_PHYSICAL),
            margin: UiRect::horizontal(px(-10)),
            padding: UiRect::horizontal(px(PIN_ROW_PADDING)),
        }
        Children [
            Text(label)
            TextFont { font_size: FontSize::Px(13.0) }
            TextColor(text)
            tag
            Pickable { should_block_lower: false, is_hoverable: false }
            --
            {editor}
            --
            @pin_port(port, color, key)
        ]
    }
}

/// A pin's port: bevy_noodle's kit port of [`PIN_PHYSICAL`] / [`PIN_VISUAL`] size, with the model key. Its hover
/// look (design §4.8) is bevy_noodle's `PortHoverStyle`; hit testing, wire drags and drop targets stay bevy_noodle's
/// own: the physical box is the hover / press area (no separate pickable area over it, decision 0033), and a dragged
/// wire takes the nearest pin within [`WIRE_TARGET_RADIUS`] (`CanvasInteraction::wire_target_radius`, set in
/// `attach_canvas`).
fn pin_port(port: Port, color: Color, key: PinKey) -> impl Scene {
    let size = kit::PortSize { physical: PIN_PHYSICAL, visual: PIN_VISUAL };
    bsn! { @kit::port_sized(port, color, size) key }
}

/// A pin's physical size (design §6.3): its port's box, where it is hovered and pressed (also while hovered: it does
/// not grow), where wires are anchored, and the room it takes (a pin row is as tall; node sizes derive from it).
pub const PIN_PHYSICAL: f32 = 24.0;
/// A pin's visual size (design §6.3): only the drawn dot, centred in the physical box, its border scaled with it;
/// wires are drawn up to its rim and the hover look grows it.
pub const PIN_VISUAL: f32 = 16.0;
/// Horizontal padding of a pin row: the kit's 10 px plus what the physical box reaches further into the node than the
/// kit's port, so a label keeps the kit's distance from it.
const PIN_ROW_PADDING: f32 = 10.0 + (PIN_PHYSICAL - kit::PORT_SIZE) / 2.0;
/// A dragged or lifted wire snaps to the nearest pin it may end on within this many logical screen px (design §6.3).
pub const WIRE_TARGET_RADIUS: f32 = 40.0;

/// The node a material previews (decision 0022).
fn previewed_node(asset: &Asset) -> Option<&str> {
    asset.preview.node.as_deref().filter(|_| matches!(asset.kind, AssetKind::Material(_)))
}

/// Scene of one model node; `routes`: the asset's [`route_declarations`].
fn node_scene(
    editor: &Editor,
    asset: &Asset,
    path: &str,
    id: &str,
    routes: &[(NodeId, String)],
    canvas: Entity,
) -> Option<Box<dyn Scene>> {
    let n = asset.nodes.get(id)?;
    let pos = Vec2::new(n.pos.0, n.pos.1);
    let key = NodeKey(id.to_string());
    let def = editor.registry.get(&n.kind);
    let sig = node_signature(asset, id, &editor.registry, &editor.functions, path);
    // The previewed node: warning icon in the title (decision 0022), pulsing outline (`node_outlines`).
    let previewed = previewed_node(asset) == Some(id);
    if def.is_some_and(|d| d.role() == NodeRole::Reroute) {
        let ty = sig.as_ref().and_then(|s| s.outputs.first()).map_or(PinType::Any, |p| p.ty);
        let c = pin_color(ty);
        let kin = PinKey { node: id.into(), pin: "in".into(), output: false };
        let kout = PinKey { node: id.into(), pin: "out".into(), output: true };
        return Some(Box::new(bsn! {
            GraphNode
            NodePosition(pos)
            key
            // A pill fitting its two pins: their physical boxes sit on its ends (centred on the border), with a
            // physical size of body between them to grab the node by.
            Node {
                width: px(2.0 * PIN_PHYSICAL),
                height: px(PIN_PHYSICAL),
                border: UiRect::all(px(1.5)),
                border_radius: BorderRadius::MAX,
            }
            BackgroundColor(Color::srgb_u8(44, 46, 52))
            SelectedBorderColor { selected: SELECTED }
            ChildOf(canvas)
            Children [
                @pin_port(Port::input(PortType::ANY), c, kin)
                --
                @pin_port(Port::output(PortType::ANY), c, kout)
            ]
        }));
    }
    let title = node_title_in(editor, asset, id, routes);
    let head = header_color(&editor.registry, asset, id, routes);
    let mut rows: Vec<Box<dyn Scene>> = Vec::new();
    // Vector editors (a number field per component) need a wider node.
    let mut wide = false;
    let params = editor.registry.resolved_params(&n.kind, &n.params);
    let previews = def.map(|d| node_preview::previews(d.as_ref(), &id.to_string())).unwrap_or_default();
    rows.extend(previews.rows);
    let mut beside = previews.beside_outputs;
    if let Some((key, inline)) = inline_for_param(&n.kind, &params) {
        wide |= matches!(inline, Inline::Vector(_));
        let v = params.get(&key).cloned().unwrap_or(Value::Float(0.0));
        let target = PropTarget::Param { node: id.into(), key };
        rows.push(Box::new(bsn! { Node { min_height: px(26), align_items: AlignItems::Center } Children [ @{inline_scene(target, inline, &v)} ] }));
    }
    if let Some(sig) = &sig {
        let mut outputs: Vec<Box<dyn Scene>> = Vec::new();
        for p in &sig.outputs {
            let key = PinKey { node: id.into(), pin: p.id.clone(), output: true };
            let label = if p.label.is_empty() { p.ty.label().to_string() } else { p.label.clone() };
            outputs.push(Box::new(pin_row(label, Port::output(PortType::ANY), pin_color(p.ty), key, LABEL, None)));
        }
        if beside.is_empty() || outputs.is_empty() {
            rows.append(&mut beside);
            rows.extend(outputs);
        } else {
            // The previews left, the output pins right (their ports stay on the node's right border).
            let cols = std::mem::take(&mut beside);
            rows.push(Box::new(bsn! {
                Node { column_gap: px(12), align_items: AlignItems::Center }
                Children [
                    Node { flex_direction: FlexDirection::Column, row_gap: px(6) } Children [ {cols} ]
                    --
                    Node { flex_direction: FlexDirection::Column, row_gap: px(6), flex_grow: 1.0 } Children [ {outputs} ]
                ]
            }));
        }
        // A usage's input is fed by its declaration, not by a wire.
        let usage = def.is_some_and(|d| d.role() == NodeRole::RouteUsage);
        for p in sig.inputs.iter().filter(|_| !usage) {
            let key = PinKey { node: id.into(), pin: p.id.clone(), output: false };
            let connected = asset.edge_into(id, &p.id).is_some();
            let mut label = if p.label.is_empty() { p.id.clone() } else { p.label.clone() };
            let mut editor_scene = None;
            if !connected {
                if let Some(what) = &p.implicit {
                    label.push_str(&format!(" ({what})"));
                } else if !p.required
                    && let Some(inline) = inline_for_pin(p.ty, p.range)
                {
                    wide |= matches!(inline, Inline::Vector(_));
                    let v = n.inputs.get(&p.id).cloned().unwrap_or_else(|| p.default.clone()).coerce(p.ty);
                    let target = PropTarget::Input { node: id.into(), pin: p.id.clone() };
                    editor_scene = Some(inline_scene(target, inline, &v));
                }
            }
            let text = if p.inactive { LABEL_DIM } else { LABEL };
            rows.push(Box::new(pin_row(label, Port::input(PortType::ANY), pin_color(p.ty), key, text, editor_scene)));
        }
    } else {
        rows.push(Box::new(
            bsn! { Text("unknown node kind") TextColor(ERROR) TextFont { font_size: FontSize::Px(12.0) } },
        ));
    }
    let min_width = if wide { px(250) } else { px(170) };
    let title: Box<dyn Scene> = if previewed {
        Box::new(bsn! {
            // `kit::title` with the icon before the text.
            Node {
                align_items: AlignItems::Center,
                column_gap: px(6),
                padding: UiRect::axes(px(10), px(5)),
                margin: UiRect::new(px(-10), px(-10), px(0), px(2)),
                border_radius: BorderRadius::top(px(6.5)),
            }
            BackgroundColor(head)
            NodeHeader
            Children [
                @icon(IconId::Warning, 16.0, ACCENT)
                --
                Text(title)
                TextFont { font_size: FontSize::Px(14.0) }
            ]
        })
    } else {
        Box::new(bsn! { @kit::title(title) BackgroundColor(head) NodeHeader })
    };
    Some(Box::new(bsn! {
        @kit::node(pos)
        Node { min_width: min_width }
        // The 1.5 px border under the selection outline, white too.
        SelectedBorderColor { selected: SELECTED }
        key
        ChildOf(canvas)
        Children [ @{title} -- {rows} ]
    }))
}

/// Colour of a comment frame: its hue at the editor's saturation / lightness (design §4.6), else its stored colour or
/// the default.
pub fn frame_color(f: &Frame) -> Color {
    match f.hue {
        Some(hue) => Color::hsl(hue, 0.55, 0.55),
        None => {
            let [r, g, b, _] = f.color.unwrap_or([0.55, 0.62, 0.75, 1.0]);
            Color::srgb(r, g, b)
        }
    }
}

/// Border (unselected) and background of a frame of colour `c`; the title is drawn in `c`.
fn frame_look(c: Color) -> (SelectedBorderColor, BackgroundColor) {
    (SelectedBorderColor { normal: c.with_alpha(0.5), selected: Color::WHITE }, BackgroundColor(c.with_alpha(0.08)))
}

/// Height (graph px) of a comment frame's title strip, one 16 px line.
const FRAME_TITLE_HEIGHT: f32 = 20.0;

fn frame_scene(f: &Frame, canvas: Entity) -> impl Scene {
    let c = frame_color(f);
    let (x, y, w, h) = f.rect;
    let (pos, title, key) = (Vec2::new(x, y), f.title.clone(), FrameKey(f.id.clone()));
    let (border, bg) = frame_look(c);
    bsn! {
        GraphNode
        // Dragging it carries the nodes inside (bevy_noodle).
        NodeFrame
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
            // The title strip spans the frame's width (a click target even with an empty title): a double click
            // there edits the title (design §5.5); a drag from it moves the frame, as from the frame itself.
            Text(title)
            TextFont { font_size: FontSize::Px(16.0) }
            TextColor(c)
            Node { width: percent(100), min_height: px(FRAME_TITLE_HEIGHT) }
            FrameTitleStrip
        ]
    }
}

// ---------------------------------------------------------------------------------------------
// Model → canvas

/// No gesture is running and no finished move is waiting for its commit (a rebuild would put the nodes back).
fn idle(world: &mut World) -> bool {
    if world.get_resource::<PendingMoves>().is_some_and(|m| !m.entities.is_empty()) {
        return false;
    }
    // A dragged wire, or wires lifted by a Ctrl+drag (design §5.3).
    let wires = world.query_filtered::<(), Or<(With<PendingWire>, With<MovingEdges>)>>().iter(world).next().is_some();
    let pressed = world
        .get_resource::<ButtonInput<MouseButton>>()
        .is_some_and(|b| b.any_pressed([MouseButton::Left, MouseButton::Right, MouseButton::Middle]));
    !wires && !pressed && !inline_editing(world)
}

/// While the rebuild waits (see [`inline_editing`]), number fields on nodes still show the model: a color
/// picker or another field may be changing their value. The field being typed into is left alone.
fn sync_inline_fields(
    editor: Res<Editor>,
    focus: Option<Res<InputFocus>>,
    fields: Query<(Entity, &PropField, &NumberInputValue)>,
    parents: Query<&ChildOf>,
    graph: GraphQuery,
    mut commands: Commands,
) {
    if !editor.is_changed() {
        return;
    }
    let typing = focus.and_then(|f| f.get()).and_then(|f| parents.get(f).ok()).map(ChildOf::parent);
    for (e, field, shown) in &fields {
        if Some(e) == typing || graph.node_of(e).is_none() {
            continue;
        }
        let Some(v) = editor.current_value(&field.target) else { continue };
        let want = match (shown, v) {
            (NumberInputValue::I32(_), Value::Int(i)) => NumberInputValue::I32(i),
            (_, v) => NumberInputValue::F32(display_value(v.floats().get(field.comp).copied().unwrap_or(0.0))),
        };
        if *shown != want {
            commands.entity(e).insert(want);
        }
    }
}

/// Text is being typed into a field inside a node, or an editor's popup (color picker) is open: a rebuild
/// would despawn it. Focused buttons and number inputs that are not being typed into (read-only after a scrub or
/// Enter, [`typing_into`]) don't count: they keep focus after a click, and their drags are covered by the
/// pressed-button check.
fn inline_editing(world: &mut World) -> bool {
    let in_node = |world: &World, mut e: Entity| loop {
        if world.get::<NodeKey>(e).is_some() {
            return true;
        }
        match world.get::<ChildOf>(e) {
            Some(parent) => e = parent.parent(),
            None => return false,
        }
    };
    let focus = world.get_resource::<InputFocus>().and_then(InputFocus::get);
    if focus.is_some_and(|e| {
        world.get::<EditableText>(e).is_some()
            && typing_into(world.get::<TextReadWriteMode>(e), false)
            && in_node(world, e)
    }) {
        return true;
    }
    let popovers: Vec<Entity> = world.query_filtered::<Entity, With<Popover>>().iter(world).collect();
    popovers.into_iter().any(|e| in_node(world, e))
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
    let (mut selection, mut frame_sel, mut enclosed, mut previewed) = (BTreeSet::new(), None, BTreeSet::new(), None);
    if let Some(d) = editor.doc() {
        for f in &d.asset.comments {
            scenes.push(Box::new(frame_scene(f, canvas)));
        }
        let routes = route_declarations(&d.asset, &editor.registry);
        for id in d.asset.nodes.keys() {
            if let Some(s) = node_scene(editor, &d.asset, &d.id.path, id, &routes, canvas) {
                scenes.push(s);
            }
        }
        edges = d.asset.edges.clone();
        selection = d.selection.clone();
        frame_sel = d.selected_frame.clone();
        enclosed = d.enclosed_frames.clone();
        previewed = previewed_node(&d.asset).map(str::to_string);
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

    // The pin under the pointer keeps its hover look: picking sees its new port only a frame later (after layout),
    // and corrects `Hovered` then if the rebuild moved the pin away.
    let hovered: Vec<PinKey> =
        world.query::<(&PinKey, &Hovered)>().iter(world).filter(|(_, h)| h.0).map(|(k, _)| k.clone()).collect();
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
    for k in hovered {
        if let Some(&port) = ports.get(&(k.node, k.pin, k.output)) {
            world.entity_mut(port).insert(Hovered(true));
        }
    }
    let pulse: Vec<Entity> = world
        .query::<(Entity, &NodeKey)>()
        .iter(world)
        .filter(|(_, k)| Some(&k.0) == previewed.as_ref())
        .map(|(e, _)| e)
        .collect();
    for e in pulse {
        world.entity_mut(e).insert(PreviewPulse);
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
        if frame_sel.as_ref() == Some(&k.0) || enclosed.contains(&k.0) {
            picked.push(e);
        }
    }
    world.select(canvas, picked, SelectMode::Replace);
    world.resource_mut::<EditorCanvas>().shown = want;
}

/// Node emphasis outlines ([`node_outline`]): every frame, but it touches only `Outline` (the preview pulse),
/// never the model, so the canvas is not rebuilt.
fn node_outlines(
    editor: Res<Editor>,
    time: Res<Time>,
    mut nodes: Query<(Entity, &NodeKey, Has<Selected>, Has<PreviewPulse>, Option<&mut Outline>)>,
    mut commands: Commands,
) {
    let diags = editor.node_diags();
    let t = time.elapsed_secs_wrapped();
    for (e, key, selected, previewed, outline) in &mut nodes {
        let diag = diags.get(&key.0).map(|v| v.iter().any(|d| d.is_error()));
        match (node_outline(previewed, selected, diag, t), outline) {
            (Some(want), Some(mut have)) => {
                have.set_if_neq(want);
            }
            (Some(want), None) => {
                commands.entity(e).try_insert(want);
            }
            (None, Some(_)) => {
                commands.entity(e).try_remove::<Outline>();
            }
            (None, None) => {}
        }
    }
}

/// Error/warning colored pin labels (node outlines: [`node_outlines`]).
fn highlight_diagnostics(editor: Res<Editor>, mut labels: Query<(&PinLabel, &mut TextColor)>) {
    let diags = editor.node_diags();
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

/// Verdicts of [`Editor::plan_move_wires`] for the Ctrl+drag running now, by (pin it started from, pin hovered):
/// bevy_noodle asks [`MoveCheck`] for every pin of the direction when the wires are lifted, and again for the pin
/// they are dropped on. The model does not change during a gesture (a drop is planned again when applied).
#[derive(Resource, Default)]
pub struct MoveVerdicts {
    verdicts: HashMap<((String, String), (String, String)), bool>,
}

/// Each Ctrl+drag starts with no verdicts: between two the graph, or a function it calls, may have changed.
fn start_move_verdicts(_: On<Add<MovingEdges>>, mut verdicts: ResMut<MoveVerdicts>) {
    verdicts.verdicts.clear();
}

/// Lifted wires may move to a pin of the same direction exactly where the whole move would succeed (design §5.3): the
/// dry run of `Action::MoveWires` decides alone, over bevy_noodle's built-in verdict (its edits never apply here,
/// `request_move_wires`). A refused pin gets no snap and takes no drop.
fn check_move(
    mut check: On<MoveCheck>,
    keys: Query<&PinKey>,
    editor: Option<Res<Editor>>,
    ec: Option<Res<EditorCanvas>>,
    mut verdicts: ResMut<MoveVerdicts>,
) {
    if ec.as_ref().is_none_or(|c| c.syncing || c.canvas != check.canvas) {
        return;
    }
    let (Ok(from), Ok(to), Some(editor)) = (keys.get(check.from), keys.get(check.to), editor) else { return };
    let (from_pin, to_pin) = ((from.node.clone(), from.pin.clone()), (to.node.clone(), to.pin.clone()));
    let ok = *verdicts
        .verdicts
        .entry((from_pin.clone(), to_pin.clone()))
        .or_insert_with(|| editor.plan_move_wires(&from_pin, from.output, &to_pin).is_ok());
    if ok { check.allow() } else { check.reject() }
}

/// A Ctrl+drag dropped on a pin becomes one `Action::MoveWires` (one undo step, decision 0032). bevy_noodle does not
/// move the edges itself: the model is the source of truth, and the canvas is rebuilt from it (decision 0008).
fn request_move_wires(
    mut request: On<EditRequested>,
    keys: Query<&PinKey>,
    ec: Option<Res<EditorCanvas>>,
    mut actions: ResMut<Actions>,
) {
    let GraphEdit::MoveEdges { from, to, .. } = &request.edit else { return };
    if ec.is_none_or(|c| c.canvas != request.canvas) {
        return;
    }
    if let (Ok(from), Ok(to)) = (keys.get(*from), keys.get(*to)) {
        actions.push(Action::MoveWires {
            node: from.node.clone(),
            pin: from.pin.clone(),
            output: from.output,
            to: (to.node.clone(), to.pin.clone()),
        });
    }
    request.reject();
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
        if msg.canvas != ec.canvas || msg.origin != EditOrigin::Interaction {
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
            // Moves of wires never apply on the canvas: `request_move_wires` turns them into `Action::MoveWires`.
            GraphChange::Deleted { .. } | GraphChange::EdgesMoved { .. } => {}
        }
    }
    // A connect into a used input reports the replaced edge first; the model connect replaces it anyway.
    out.retain(|a| !matches!(a, Action::Disconnect { to } if connected_inputs.contains(to)));
    actions.0.extend(out);
}

/// Commits finished moves (a frame's drag lists the nodes it carries) as one action, a frame later.
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
            n.push((k.0.clone(), tidy(p.0.into())));
        } else if let Ok((k, p)) = frames.get(e) {
            f.push((k.0.clone(), tidy(p.0.into())));
        }
    }
    n.sort_by(|a, b| a.0.cmp(&b.0));
    actions.push(Action::Moved { nodes: n, frames: f });
}

fn sync_selection(
    ec: Option<Res<EditorCanvas>>,
    editor: Res<Editor>,
    selected: Query<(Option<&NodeKey>, Option<&FrameKey>), With<Selected>>,
    boxes: Query<(&SelectionBox, &CanvasView)>,
    mut actions: ResMut<Actions>,
) {
    let Some(ec) = ec else { return };
    let Some(d) = editor.doc() else { return };
    if ec.shown.as_ref().map(|s| &s.0) != Some(&d.id) {
        return;
    }
    let mut nodes = BTreeSet::new();
    let mut frames = BTreeSet::new();
    for (n, f) in &selected {
        if let Some(n) = n {
            nodes.insert(n.0.clone());
        }
        if let Some(f) = f {
            frames.insert(f.0.clone());
        }
    }
    // Frames selected together with nodes (design §4.5): while a selection box is dragged, those it fully encloses
    // (bevy_noodle selects every frame it touches); afterwards the marked ones as long as they stay selected.
    let marked = match boxes.get(ec.canvas) {
        Ok((b, view)) => {
            let area = Rect::from_corners(view.canvas_to_graph(b.0.min), view.canvas_to_graph(b.0.max));
            let inside = |f: &&Frame| {
                let (x, y, w, h) = f.rect;
                area.contains(Vec2::new(x, y)) && area.contains(Vec2::new(x + w, y + h))
            };
            d.asset.comments.iter().filter(|f| frames.contains(&f.id)).filter(inside).map(|f| f.id.clone()).collect()
        }
        Err(_) => frames.intersection(&d.enclosed_frames).cloned().collect(),
    };
    let enclosed = d.enclosed(&marked, &nodes);
    if nodes != d.selection {
        actions.push(Action::Select(nodes.clone()));
        if !enclosed.is_empty() {
            actions.push(Action::SelectFrames(enclosed));
        }
    } else if enclosed != d.enclosed_frames {
        actions.push(Action::SelectFrames(enclosed));
    }
    let frame = frames.into_iter().next();
    if nodes.is_empty() && frame != d.selected_frame {
        actions.push(Action::SelectFrame(frame));
    }
}

/// Comment frame colours follow the model without a rebuild (a hue drag recolours live; the canvas is rebuilt
/// only after the gesture).
fn frame_colors(
    editor: Res<Editor>,
    mut frames: Query<(&FrameKey, &mut SelectedBorderColor, &mut BackgroundColor, &Children)>,
    mut titles: Query<&mut TextColor>,
) {
    if !editor.is_changed() {
        return;
    }
    let Some(d) = editor.doc() else { return };
    for (key, mut border, mut bg, children) in &mut frames {
        let Some(f) = d.asset.comments.iter().find(|f| f.id == key.0) else { continue };
        let c = frame_color(f);
        let (want_border, want_bg) = frame_look(c);
        border.set_if_neq(want_border);
        bg.set_if_neq(want_bg);
        for child in children {
            if let Ok(mut t) = titles.get_mut(*child) {
                t.set_if_neq(TextColor(c));
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// View

/// Converts a window position (logical px) into graph space of `canvas`. Same math as bevy_noodle's `local()`:
/// the node's scale factor is the window's times `UiScale`, window positions are scaled by the window's only.
pub fn window_to_graph(
    world_pos: Vec2,
    computed: &ComputedNode,
    transform: &UiGlobalTransform,
    view: &CanvasView,
    ui_scale: f32,
) -> Option<Vec2> {
    let scale = computed.inverse_scale_factor();
    let n = computed.normalize_point(*transform, world_pos / (scale * ui_scale))?;
    let local = (n + 0.5) * computed.size() * scale;
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

/// Graph-space center of the visible canvas (its view and laid-out node).
pub fn view_center(view: &CanvasView, c: &ComputedNode) -> Vec2 {
    view.canvas_to_graph(c.size() * c.inverse_scale_factor() / 2.0)
}

/// Link for a palette opened from a wire drop.
pub fn link_of(key: &PinKey) -> Link {
    Link { node: key.node.clone(), pin: key.pin.clone(), from_output: key.output }
}

pub fn node_ids(world: &mut World) -> Vec<(Entity, NodeId)> {
    world.query::<(Entity, &NodeKey)>().iter(world).map(|(e, k)| (e, k.0.clone())).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::math::Affine2;

    /// With `UiScale` 2 the canvas node's scale factor is 2 (window scale 1), so its logical size is half its
    /// physical size; window positions are not scaled by `UiScale` (bevy_noodle's `local()`).
    #[test]
    fn window_to_graph_respects_ui_scale() {
        let computed = ComputedNode { size: Vec2::new(800.0, 600.0), inverse_scale_factor: 0.5, ..default() };
        let transform = UiGlobalTransform::from(Affine2::from_translation(Vec2::new(400.0, 300.0)));
        let view = CanvasView::default();
        // The window point (200, 100) is 1/4 of the width and 1/6 of the height into the canvas (logical 400x300).
        let at = window_to_graph(Vec2::new(200.0, 100.0), &computed, &transform, &view, 2.0).unwrap();
        let expected = view.canvas_to_graph(Vec2::new(100.0, 50.0));
        assert!((at - expected).length() < 0.01, "{at} != {expected}");
        // Without scaling the same call matches the old behaviour.
        let computed1 = ComputedNode { inverse_scale_factor: 1.0, ..computed };
        let at1 = window_to_graph(Vec2::new(200.0, 100.0), &computed1, &transform, &view, 1.0).unwrap();
        assert!((at1 - view.canvas_to_graph(Vec2::new(200.0, 100.0))).length() < 0.01);
    }

    #[test]
    fn name_hue_is_stable() {
        let h = name_hue("albedo");
        assert_eq!(h, name_hue("albedo"));
        assert!((0.0..360.0).contains(&h), "{h}");
        assert_ne!(h, name_hue("normal"));
        // FNV-1a of no bytes is the offset basis: pins the algorithm.
        assert_eq!(name_hue(""), (2166136261u32 % 360) as f32);
    }

    fn material() -> Asset {
        use shader_graph_core::model::{MaterialKind, PbrSettings};
        let mut a = Asset::new("m", AssetKind::Material(MaterialKind::Pbr3d(PbrSettings::default())));
        a.nodes.clear();
        a
    }

    /// [`header_color`] with the asset's routes.
    fn color(reg: &Registry, a: &Asset, id: &str) -> Color {
        header_color(reg, a, id, &route_declarations(a, reg))
    }

    #[test]
    fn route_usage_shares_declaration_header() {
        use shader_graph_core::model::Node;
        let reg = shader_stdlib::registry();
        let mut a = material();
        let decl = a.add_node(Node::new("stdlib::NamedRoute", (0.0, 0.0)).with_param("name", Value::Str("wet".into())));
        let usage = a.add_node(
            Node::new("stdlib::NamedRouteUsage", (0.0, 0.0)).with_param(ROUTE_PARAM, Value::Str(decl.clone())),
        );
        let c = color(&reg, &a, &decl);
        assert_eq!(c, name_header_color("wet"));
        assert_eq!(color(&reg, &a, &usage), c);
        // A usage of no declaration keeps the category colour.
        let lost = a.add_node(Node::new("stdlib::NamedRouteUsage", (0.0, 0.0)));
        assert_eq!(color(&reg, &a, &lost), category_color("Utility"));
    }

    #[test]
    fn unlabelled_constant_keeps_category_colour() {
        use shader_graph_core::model::Node;
        let reg = shader_stdlib::registry();
        let mut a = material();
        let plain = a.add_node(Node::new("stdlib::Constant", (0.0, 0.0)));
        assert_eq!(color(&reg, &a, &plain), category_color("Constants"));
        let mut labelled = Node::new("stdlib::Constant3Vector", (0.0, 0.0));
        labelled.label = "tint".into();
        let labelled = a.add_node(labelled);
        assert_eq!(color(&reg, &a, &labelled), name_header_color("tint"));
        // Global*: by the constant's name, set or not.
        let global = a.add_node(
            Node::new("stdlib::GlobalConstF32", (0.0, 0.0)).with_param("const", Value::Str("roughness_base".into())),
        );
        assert_eq!(color(&reg, &a, &global), name_header_color("roughness_base"));
        let unset = a.add_node(Node::new("stdlib::GlobalConstF32", (0.0, 0.0)));
        assert_eq!(color(&reg, &a, &unset), category_color("Constants"));
    }

    #[test]
    fn outline_priority() {
        // Previewed: 6 px, pulsing; wins over selection and diagnostics.
        let o = node_outline(true, true, Some(true), 0.0).unwrap();
        assert_eq!((o.width, o.color), (px(6), pulse_color(0.0)));
        // Selected: 3 px, diagnostic colour first.
        assert_eq!(node_outline(false, true, None, 0.0).unwrap().color, SELECTED);
        let o = node_outline(false, true, Some(false), 0.0).unwrap();
        assert_eq!((o.width, o.color), (px(3), WARNING));
        // Diagnostics alone: today's 2 px.
        let o = node_outline(false, false, Some(true), 0.0).unwrap();
        assert_eq!((o.width, o.color), (px(2), ERROR));
        assert!(node_outline(false, false, None, 0.0).is_none());
    }

    /// The system itself, not only [`node_outline`]: a selected node gets the 3 px selection outline, the previewed
    /// one the 6 px pulse, a plain node none (a stale outline is removed).
    #[test]
    fn node_outlines_selected_and_previewed() {
        use bevy::ecs::system::RunSystemOnce;
        let mut world = World::new();
        world.insert_resource(Editor::default());
        world.insert_resource(Time::<()>::default());
        let selected = world.spawn((NodeKey("a".into()), Selected)).id();
        let previewed = world.spawn((NodeKey("b".into()), PreviewPulse)).id();
        let plain = world.spawn(NodeKey("c".into())).id();
        let stale = world.spawn((NodeKey("d".into()), Outline::new(px(3), px(0), SELECTED))).id();
        world.run_system_once(node_outlines).unwrap();
        let outline = |e: Entity| world.get::<Outline>(e).copied();
        let o = outline(selected).expect("a selected node is outlined");
        assert_eq!((o.width, o.offset, o.color), (px(3), px(0), SELECTED));
        let o = outline(previewed).expect("the previewed node is outlined");
        assert_eq!((o.width, o.offset, o.color), (px(6), px(0), pulse_color(0.0)));
        assert!(outline(plain).is_none());
        assert!(outline(stale).is_none(), "an outline nothing asks for is removed");
    }

    #[test]
    fn preview_pulse_swings_between_orange_and_dark_red() {
        let orange = Color::srgb_u8(255, 140, 0).to_srgba();
        let red = Color::srgb_u8(140, 20, 20).to_srgba();
        let close = |a: Color, b: Srgba| (a.to_srgba().to_vec4() - b.to_vec4()).length() < 1e-3;
        assert!(close(pulse_color(0.0), orange));
        assert!(close(pulse_color(PULSE_PERIOD / 2.0), red));
        assert!(close(pulse_color(PULSE_PERIOD), orange));
        let mid = pulse_color(PULSE_PERIOD / 4.0).to_srgba();
        assert!(mid.red < orange.red && mid.red > red.red, "{mid:?}");
    }
}
