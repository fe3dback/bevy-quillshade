//! The bevy_noodle canvas of the active graph document of a [`GraphDomain`].
//!
//! One-way sync (spec §14): the model is the source of truth and the canvas is rebuilt from it ([`rebuild_canvas`]);
//! user gestures arrive as `EditApplied` (origin `Interaction`) and become [`GraphEdit`]s.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::marker::PhantomData;

use bevy::feathers::controls::NumberInputValue;
use bevy::input_focus::InputFocus;
use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::text::{EditableText, TextReadWriteMode};
use bevy::ui::Selected;
use bevy::ui::ui_transform::UiGlobalTransform;
use bevy::ui_widgets::popover::Popover;
use bevy_noodle::prelude::GraphEdit as NoodleEdit;
use bevy_noodle::prelude::*;
use bevy_noodle::style::kit;
use editor_core::action::Actions;
use editor_core::doc_kind::TabDoc;
use editor_core::dynval::{PropValue, display_value};
use editor_core::editors::ViewOf;
use editor_core::keys::typing_into;
use editor_core::nav::NavView;
use editor_core::state::editor::Editor;
use editor_core::tab::{TabKey, ViewKindId};
use editor_core::theme::ACCENT;
use editor_core::ui::icons::{IconId, icon};
use editor_core::ui::props::PropField;

use crate::domain::{GraphCtxRef, GraphDomain, active};
use crate::edit::{GraphEdit, PinKey};
use crate::layout::{GRID_SNAP, enclosed, tidy};
use crate::view::{FrameView, NodeLook, NodeShape, NodeView, PinLook, PreviewRow};

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

/// Which graph view (the [`GraphDomain::VIEW`] of its domain) a canvas entity belongs to: on every node and frame
/// root, pin port and pin label the canvas spawns, so a domain's systems touch only its own entities when several
/// graph domains are registered.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct GraphOf(pub ViewKindId);

impl Default for GraphOf {
    fn default() -> Self {
        GraphOf(ViewKindId(""))
    }
}

/// The views of every registered graph domain ([`crate::register_graph`]): a camera request is dropped only when the
/// active tab is none of them.
#[derive(Resource, Default, Clone, Debug)]
pub struct GraphViews(pub Vec<ViewKindId>);

/// The graph canvas entity of a view ([`EditorCanvas`] is attached to it): the one tagged `ViewOf(G::VIEW)`, or one
/// without a view tag (a test spawning the bare canvas).
#[derive(Component, Default, Clone)]
pub struct EditorCanvasMarker;

/// The canvas of `G` and what it shows.
#[derive(Resource)]
pub struct EditorCanvas<G: GraphDomain> {
    pub canvas: Entity,
    /// Document (its tab key) and revision currently on the canvas.
    pub shown: Option<(TabKey, u64)>,
    /// Saved pan/zoom per document.
    views: HashMap<TabKey, (Vec2, f32)>,
    /// True while the canvas replays the model (connection checks are skipped).
    pub syncing: bool,
    graph: PhantomData<fn() -> G>,
}

impl<G: GraphDomain> EditorCanvas<G> {
    pub fn new(canvas: Entity) -> Self {
        EditorCanvas { canvas, shown: None, views: HashMap::new(), syncing: false, graph: PhantomData }
    }
}

/// Moved entities waiting to be committed as one undo step.
#[derive(Resource)]
pub struct PendingMoves<G> {
    entities: HashSet<Entity>,
    quiet_frames: u8,
    graph: PhantomData<fn() -> G>,
}

impl<G> Default for PendingMoves<G> {
    fn default() -> Self {
        PendingMoves { entities: HashSet::new(), quiet_frames: 0, graph: PhantomData }
    }
}

/// The canvas sync of `G` (`DomainRegistrar::logic`, also headless).
pub struct CanvasPlugin<G>(PhantomData<fn() -> G>);

impl<G> Default for CanvasPlugin<G> {
    fn default() -> Self {
        CanvasPlugin(PhantomData)
    }
}

impl<G: GraphDomain> Plugin for CanvasPlugin<G> {
    fn build(&self, app: &mut App) {
        app.init_resource::<PendingMoves<G>>()
            .init_resource::<MoveVerdicts<G>>()
            .init_resource::<SettleState<G>>()
            .init_resource::<CanvasMarks<G>>()
            .add_systems(
                Update,
                (
                    sync_from_canvas::<G>,
                    commit_moves::<G>,
                    sync_selection::<G>,
                    rebuild_canvas::<G>,
                    frame_colors::<G>,
                    sync_inline_fields::<G>,
                    collect_marks::<G>,
                    highlight_diagnostics::<G>,
                    node_outlines::<G>,
                    keep_center_on_resize::<G>,
                    apply_view_requests::<G>,
                    report_settled_view::<G>,
                )
                    .chain()
                    .in_set(CanvasSystems),
            )
            .add_observer(check_connection::<G>)
            .add_observer(start_move_verdicts::<G>)
            .add_observer(check_move::<G>)
            .add_observer(request_move_wires::<G>);
    }
}

#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct CanvasSystems;

/// Inserts [`EditorCanvas`] once the canvas entity exists. RMB drag pans (UE-like); MMB pan is handled in
/// [`crate::input`], RMB click without a drag opens the palette.
pub fn attach_canvas<G: GraphDomain>(world: &mut World) {
    if world.contains_resource::<EditorCanvas<G>>() {
        return;
    }
    let found = world
        .query_filtered::<(Entity, Option<&ViewOf>), With<EditorCanvasMarker>>()
        .iter(world)
        .find(|(_, v)| v.is_none_or(|v| v.0 == G::VIEW))
        .map(|(e, _)| e);
    if let Some(canvas) = found {
        if let Some(mut interaction) = world.get_mut::<CanvasInteraction>(canvas) {
            interaction.pan_button = Some(PointerButton::Secondary);
            // UE: dragging from a connected input starts a new wire; Ctrl+drag lifts the pin's wires to move them
            // (decision 0032, design §5.3), "Ctrl" being Ctrl or Super as in the hotkeys.
            interaction.detach_wires = false;
            interaction.move_wires_keys =
                vec![KeyCode::ControlLeft, KeyCode::ControlRight, KeyCode::SuperLeft, KeyCode::SuperRight];
            // Dragged nodes snap to the grid (design §4.1); comment frames carry their nodes (`frame_scene`).
            interaction.snap = Some(GRID_SNAP);
            // A dragged or lifted wire takes the nearest pin it may end on within 40 screen px (design §6.3).
            interaction.wire_target_radius = Some(WIRE_TARGET_RADIUS);
        }
        // A hovered pin's dot grows and lightens (design §4.8), also under a dragged wire.
        let hover = PortHoverStyle { scale: PIN_HOVER_SCALE, lighten: PIN_HOVER_LIGHTEN };
        world.entity_mut(canvas).insert(hover);
        world.insert_resource(EditorCanvas::<G>::new(canvas));
    }
}

/// The graph editor's view of `G`: the bevy_noodle canvas.
pub fn graph_view<G: GraphDomain>() -> impl Scene {
    let view = ViewOf(G::VIEW);
    bsn! {
        @kit::canvas() EditorCanvasMarker view
        // Dimmer than bevy_noodle's default (design §4.9).
        bevy_noodle::style::CanvasGrid {
            minor: Color::srgba(1.0, 1.0, 1.0, 0.006),
            major: Color::srgba(1.0, 1.0, 1.0, 0.014),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Look

/// Selected node outline: white like a selected comment frame's border (design §2), so it never reads as [`WARNING`].
pub const SELECTED: Color = Color::WHITE;
/// Period of the previewed node's outline pulse, seconds (design §4.4).
pub const PULSE_PERIOD: f32 = 1.2;

/// Previewed node outline colour at `t` seconds: a smooth sine between bright orange (t = 0) and dark red.
pub fn pulse_color(t: f32) -> Color {
    let k = 0.5 - 0.5 * (t * std::f32::consts::TAU / PULSE_PERIOD).cos();
    let (a, b) = (Color::srgb_u8(255, 140, 0).to_srgba(), Color::srgb_u8(140, 20, 20).to_srgba());
    Color::Srgba(a.mix(&b, k))
}

/// Emphasis outline of a node (design §4.3, §4.4): previewed 6 px pulsing; selected 3 px (diagnostic colour if
/// any); diagnostics alone 2 px. `diag`: worst diagnostic, `Some(true)` for an error. The node border stays 1.5 px:
/// an outline changes no layout, so ports and wires don't move.
pub fn node_outline(previewed: bool, selected: bool, diag: Option<bool>, t: f32) -> Option<Outline> {
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
/// … and this much lighter (bevy_noodle's `PortHoverStyle`, set on the canvas in [`attach_canvas`]).
pub const PIN_HOVER_LIGHTEN: f32 = 0.15;

pub const ERROR: Color = Color::srgb(0.95, 0.30, 0.30);
pub const WARNING: Color = Color::srgb(0.95, 0.75, 0.25);
const LABEL: Color = Color::srgb(0.84, 0.85, 0.88);
const LABEL_DIM: Color = Color::srgb(0.45, 0.46, 0.50);

fn pin_row(
    label: String,
    port: Port,
    color: Color,
    key: PinKey,
    text: Color,
    editor: Option<Box<dyn Scene>>,
    owner: GraphOf,
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
            owner
            Pickable { should_block_lower: false, is_hoverable: false }
            --
            {editor}
            --
            @pin_port(port, color, key, owner)
        ]
    }
}

/// A pin's port: bevy_noodle's kit port of [`PIN_PHYSICAL`] / [`PIN_VISUAL`] size, with the model key. Its hover
/// look (design §4.8) is bevy_noodle's `PortHoverStyle`; hit testing, wire drags and drop targets stay bevy_noodle's
/// own: the physical box is the hover / press area (no separate pickable area over it, decision 0033), and a dragged
/// wire takes the nearest pin within [`WIRE_TARGET_RADIUS`] (`CanvasInteraction::wire_target_radius`, set in
/// [`attach_canvas`]).
fn pin_port(port: Port, color: Color, key: PinKey, owner: GraphOf) -> impl Scene {
    let size = kit::PortSize { physical: PIN_PHYSICAL, visual: PIN_VISUAL };
    bsn! { @kit::port_sized(port, color, size) key owner }
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

/// The key of a pin of node `node`.
fn pin_key(node: &str, p: &PinLook) -> PinKey {
    PinKey { node: node.into(), pin: p.id.clone(), output: p.output }
}

/// Scene of one model node from its look and preview rows.
fn node_scene(
    n: &NodeView,
    look: NodeLook,
    previews: Vec<PreviewRow>,
    canvas: Entity,
    owner: GraphOf,
) -> Box<dyn Scene> {
    let (id, pos) = (n.id.as_str(), n.pos);
    let key = NodeKey(id.to_string());
    let comment = look.comment.unwrap_or_default();
    if look.shape == NodeShape::Pill {
        let input = look.pins.iter().find(|p| !p.output);
        let output = look.pins.iter().find(|p| p.output);
        let color = |p: Option<&PinLook>| p.map_or(Color::WHITE, |p| p.color);
        let (cin, cout) = (color(input), color(output));
        let kin = input.map_or_else(|| PinKey { node: id.into(), pin: "in".into(), output: false }, |p| pin_key(id, p));
        let kout =
            output.map_or_else(|| PinKey { node: id.into(), pin: "out".into(), output: true }, |p| pin_key(id, p));
        return Box::new(bsn! {
            GraphNode
            NodePosition(pos)
            key
            owner
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
                @pin_port(Port::input(PortType::ANY), cin, kin, owner)
                --
                @pin_port(Port::output(PortType::ANY), cout, kout, owner)
                --
                {node_comment(&comment)}
            ]
        });
    }
    let (title, head) = (look.title, look.header);
    let mut rows: Vec<Box<dyn Scene>> = Vec::new();
    let mut beside: Vec<Box<dyn Scene>> = Vec::new();
    for p in previews {
        if p.beside_outputs { &mut beside } else { &mut rows }.push(p.scene);
    }
    if let Some(body) = look.body {
        rows.push(Box::new(bsn! { Node { min_height: px(26), align_items: AlignItems::Center } Children [ @{body} ] }));
    }
    if let Some(note) = look.note {
        rows.push(Box::new(bsn! { Text(note) TextColor(ERROR) TextFont { font_size: FontSize::Px(12.0) } }));
    } else {
        let mut inline: HashMap<String, Box<dyn Scene>> = look.inline.into_iter().collect();
        let mut outputs: Vec<Box<dyn Scene>> = Vec::new();
        for p in look.pins.iter().filter(|p| p.output && !p.hidden) {
            let row =
                pin_row(p.label.clone(), Port::output(PortType::ANY), p.color, pin_key(id, p), LABEL, None, owner);
            outputs.push(Box::new(row));
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
        for p in look.pins.iter().filter(|p| !p.output && !p.hidden) {
            let text = if p.inactive { LABEL_DIM } else { LABEL };
            let editor = inline.remove(&p.id);
            let row =
                pin_row(p.label.clone(), Port::input(PortType::ANY), p.color, pin_key(id, p), text, editor, owner);
            rows.push(Box::new(row));
        }
    }
    let min_width = if look.wide { px(250) } else { px(170) };
    let title: Box<dyn Scene> = if look.emphasis.pulse {
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
    Box::new(bsn! {
        @kit::node(pos)
        Node { min_width: min_width }
        // The 1.5 px border under the selection outline, white too.
        SelectedBorderColor { selected: SELECTED }
        key
        owner
        ChildOf(canvas)
        Children [ @{title} -- {rows} -- {node_comment(&comment)} ]
    })
}

/// Background of a [`node_comment`] and its arrow.
const COMMENT_BG: Color = Color::srgba(0.0, 0.0, 0.0, 0.6);
/// Height (graph px) of a [`node_comment`]'s arrow; the gap between the comment and the node is one px more.
const COMMENT_ARROW: f32 = 5.0;
/// The node's border (`kit::node`): the comment spans the node's outer width.
const NODE_BORDER: f32 = 1.5;

/// A node's comment, nothing when it has none: a rounded translucent black box of small white word-wrapped text right
/// above the node, as wide as the node and as tall as its text, with a small arrow down to the node. Absolute, so the
/// node's layout and size ignore it; picking passes through it.
fn node_comment(comment: &str) -> Vec<Box<dyn Scene>> {
    if comment.is_empty() {
        return Vec::new();
    }
    let comment = comment.to_string();
    let edge = px(-NODE_BORDER);
    vec![Box::new(bsn! {
        Node {
            position_type: PositionType::Absolute,
            left: edge,
            right: edge,
            // From the top of the node's padding box: over its top border, then the gap.
            bottom: percent(100),
            padding: UiRect::bottom(px(NODE_BORDER + 1.0)),
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
        }
        Pickable::IGNORE
        Children [
            Node {
                align_self: AlignSelf::Stretch,
                padding: UiRect::axes(px(6), px(4)),
                border_radius: BorderRadius::all(px(5)),
            }
            BackgroundColor(COMMENT_BG)
            Pickable::IGNORE
            Children [
                Text(comment)
                TextFont { font_size: FontSize::Px(10.0) }
                TextColor(Color::WHITE)
                TextLayout { linebreak: LineBreak::WordOrCharacter }
                Pickable::IGNORE
            ]
            --
            // A triangle: a zero-size box whose top border is the only coloured side (Bevy joins the sides
            // diagonally).
            Node { width: px(0), height: px(0), border: UiRect::new(px(COMMENT_ARROW), px(COMMENT_ARROW), px(COMMENT_ARROW), px(0)) }
            BorderColor { top: COMMENT_BG, left: Color::NONE, right: Color::NONE, bottom: Color::NONE }
            Pickable::IGNORE
        ]
    })]
}

/// Colour of a comment frame: its hue at the editor's saturation / lightness (design §4.6), else its stored colour or
/// the default.
pub fn frame_color(f: &FrameView) -> Color {
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

fn frame_scene(f: &FrameView, canvas: Entity, owner: GraphOf) -> impl Scene {
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
        owner
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

/// The active `G` document as the canvas keys it: its tab and revision.
fn shown_key<G: GraphDomain>(editor: &Editor) -> Option<(TabKey, u64)> {
    let d = active::<G>(editor)?;
    Some((editor.active_key()?, d.revision()))
}

/// No gesture is running and no finished move is waiting for its commit (a rebuild would put the nodes back).
pub fn idle<G: GraphDomain>(world: &mut World) -> bool {
    if world.get_resource::<PendingMoves<G>>().is_some_and(|m| !m.entities.is_empty()) {
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
fn sync_inline_fields<G: GraphDomain>(
    editor: Res<Editor>,
    focus: Option<Res<InputFocus>>,
    fields: Query<(Entity, &PropField, &NumberInputValue)>,
    parents: Query<&ChildOf>,
    owners: Query<&GraphOf>,
    graph: GraphQuery,
    mut commands: Commands,
) {
    if !editor.is_changed() || active::<G>(&editor).is_none() {
        return;
    }
    let ours = |node: Entity| owners.get(node).is_ok_and(|g| g.0 == G::VIEW);
    let typing = focus.and_then(|f| f.get()).and_then(|f| parents.get(f).ok()).map(ChildOf::parent);
    for (e, field, shown) in &fields {
        if Some(e) == typing || !graph.node_of(e).is_some_and(ours) {
            continue;
        }
        let Some(v) = editor.current_value(&field.target) else { continue };
        let want = match (shown, v) {
            (NumberInputValue::I32(_), PropValue::Int(i)) => NumberInputValue::I32(i),
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
pub fn rebuild_canvas<G: GraphDomain>(world: &mut World) {
    let Some(ec) = world.get_resource::<EditorCanvas<G>>() else { return };
    let (canvas, shown) = (ec.canvas, ec.shown.clone());
    let editor = world.resource::<Editor>();
    let want = shown_key::<G>(editor);
    if want == shown {
        return;
    }
    let same_doc = want.as_ref().map(|w| &w.0) == shown.as_ref().map(|s| &s.0);
    if same_doc && !idle::<G>(world) {
        return;
    }

    // Collect everything from the model first (observers below read `Editor`).
    let editor = world.resource::<Editor>();
    let mut scenes: Vec<Box<dyn Scene>> = Vec::new();
    let mut edges = Vec::new();
    let (mut selection, mut frame_sel, mut enclosed, mut previewed) = (BTreeSet::new(), None, BTreeSet::new(), None);
    if let Some(d) = active::<G>(editor) {
        let ctx = GraphCtxRef { editor };
        let view = G::view(d);
        for f in &view.frames {
            scenes.push(Box::new(frame_scene(f, canvas, GraphOf(G::VIEW))));
        }
        for n in &view.nodes {
            let look = G::node_look(&ctx, d, &n.id);
            if look.emphasis.pulse {
                previewed = Some(n.id.clone());
            }
            scenes.push(node_scene(n, look, G::previews(&ctx, d, &n.id), canvas, GraphOf(G::VIEW)));
        }
        edges = view.edges;
        selection = view.selection.clone();
        frame_sel = view.selected_frame.map(str::to_string);
        enclosed = view.enclosed_frames.clone();
    }

    // View per document.
    if !same_doc {
        let view = world.get::<CanvasView>(canvas).map(|v| (v.pan, v.zoom));
        // The outgoing graph's last view goes into its history entry, so Back returns there (decision 0041); it may
        // not have settled yet. A canvas that is not laid out has no centre.
        let outgoing = world
            .get::<CanvasView>(canvas)
            .zip(world.get::<ComputedNode>(canvas))
            .filter(|(_, c)| (c.size() * c.inverse_scale_factor()).min_element() > 1.0)
            .map(nav_view);
        if let (Some((old, _)), Some(v)) = (&shown, outgoing) {
            world.resource_mut::<Editor>().bypass_change_detection().nav.amend_last_of(old, v);
        }
        let mut ec = world.resource_mut::<EditorCanvas<G>>();
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
    // This canvas's entities only ([`GraphOf`]).
    let own = GraphOf(G::VIEW);
    let hovered: Vec<PinKey> = world
        .query::<(&PinKey, &Hovered, &GraphOf)>()
        .iter(world)
        .filter(|(_, h, g)| h.0 && **g == own)
        .map(|(k, ..)| k.clone())
        .collect();
    let old: Vec<Entity> = world
        .query_filtered::<(Entity, &GraphOf), Or<(With<NodeKey>, With<FrameKey>)>>()
        .iter(world)
        .filter(|(_, g)| **g == own)
        .map(|(e, _)| e)
        .collect();
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
    for (e, k, _) in world.query::<(Entity, &PinKey, &GraphOf)>().iter(world).filter(|(.., g)| **g == own) {
        ports.insert((k.node.clone(), k.pin.clone(), k.output), e);
    }
    for k in hovered {
        if let Some(&port) = ports.get(&(k.node, k.pin, k.output)) {
            world.entity_mut(port).insert(Hovered(true));
        }
    }
    let pulse: Vec<Entity> = world
        .query::<(Entity, &NodeKey, &GraphOf)>()
        .iter(world)
        .filter(|(_, k, g)| **g == own && Some(&k.0) == previewed.as_ref())
        .map(|(e, ..)| e)
        .collect();
    for e in pulse {
        world.entity_mut(e).insert(PreviewPulse);
    }
    world.resource_mut::<EditorCanvas<G>>().syncing = true;
    for (from, to) in &edges {
        let a = ports.get(&(from.node.clone(), from.pin.clone(), true));
        let b = ports.get(&(to.node.clone(), to.pin.clone(), false));
        if let (Some(&a), Some(&b)) = (a, b)
            && let Err(e) = world.graph_edit(canvas, NoodleEdit::Connect { from: a, to: b })
        {
            warn!(
                "cannot show edge {:?} -> {:?}: {e:?}",
                (from.node.as_str(), from.pin.as_str()),
                (to.node.as_str(), to.pin.as_str())
            );
        }
    }
    world.resource_mut::<EditorCanvas<G>>().syncing = false;

    let mut picked = Vec::new();
    for (e, k, _) in world.query::<(Entity, &NodeKey, &GraphOf)>().iter(world).filter(|(.., g)| **g == own) {
        if selection.contains(&k.0) {
            picked.push(e);
        }
    }
    for (e, k, _) in world.query::<(Entity, &FrameKey, &GraphOf)>().iter(world).filter(|(.., g)| **g == own) {
        if frame_sel.as_ref() == Some(&k.0) || enclosed.contains(&k.0) {
            picked.push(e);
        }
    }
    world.select(canvas, picked, SelectMode::Replace);
    world.resource_mut::<EditorCanvas<G>>().shown = want;
}

/// The diagnostics marks of `G`'s active graph this frame ([`GraphDomain::diagnostics`], asked once per frame by
/// [`collect_marks`]): the worst one of each node (`true`: an error) and all of them.
#[derive(Resource)]
pub struct CanvasMarks<G> {
    worst: HashMap<String, bool>,
    marks: Vec<crate::view::DiagMark>,
    graph: PhantomData<fn() -> G>,
}

impl<G> Default for CanvasMarks<G> {
    fn default() -> Self {
        CanvasMarks { worst: HashMap::new(), marks: Vec::new(), graph: PhantomData }
    }
}

/// Fills [`CanvasMarks`] (empty while `G` is not active).
fn collect_marks<G: GraphDomain>(editor: Res<Editor>, mut out: ResMut<CanvasMarks<G>>) {
    let marks = match active::<G>(&editor) {
        Some(d) => G::diagnostics(&GraphCtxRef { editor: &editor }, d),
        None => Vec::new(),
    };
    let mut worst: HashMap<String, bool> = HashMap::new();
    for m in &marks {
        *worst.entry(m.node.clone()).or_default() |= m.error;
    }
    out.worst = worst;
    out.marks = marks;
}

/// Node emphasis outlines ([`node_outline`]) of `G`'s canvas: every frame, but it touches only `Outline` (the preview
/// pulse), never the model, so the canvas is not rebuilt. Only `G`'s nodes ([`GraphOf`]): another graph domain's
/// canvas keeps its outlines.
pub fn node_outlines<G: GraphDomain>(
    marks: Option<Res<CanvasMarks<G>>>,
    time: Res<Time>,
    mut nodes: Query<(Entity, &NodeKey, &GraphOf, Has<Selected>, Has<PreviewPulse>, Option<&mut Outline>)>,
    mut commands: Commands,
) {
    let t = time.elapsed_secs_wrapped();
    for (e, key, owner, selected, previewed, outline) in &mut nodes {
        if owner.0 != G::VIEW {
            continue;
        }
        let diag = marks.as_ref().and_then(|m| m.worst.get(&key.0).copied());
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

/// Error/warning colored pin labels of `G`'s active graph (node outlines: [`node_outlines`]).
fn highlight_diagnostics<G: GraphDomain>(
    editor: Res<Editor>,
    marks: Res<CanvasMarks<G>>,
    mut labels: Query<(&PinLabel, &GraphOf, &mut TextColor)>,
) {
    if active::<G>(&editor).is_none() {
        return;
    }
    for (label, _, mut color) in labels.iter_mut().filter(|(_, g, _)| g.0 == G::VIEW) {
        let pin_diag = marks
            .marks
            .iter()
            .filter(|m| m.node == label.node && m.pin.as_deref() == Some(label.pin.as_str()))
            .map(|m| m.error)
            .reduce(|a, b| a || b);
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

/// Validates a wire against the domain's rules ([`GraphDomain::can_connect`]); runs for candidates and drops.
fn check_connection<G: GraphDomain>(
    mut check: On<ConnectionCheck>,
    keys: Query<&PinKey>,
    editor: Option<Res<Editor>>,
    ec: Option<Res<EditorCanvas<G>>>,
) {
    if ec.as_ref().is_none_or(|c| c.syncing || c.canvas != check.canvas) {
        return;
    }
    let (Ok(a), Ok(b), Some(editor)) = (keys.get(check.ports.output), keys.get(check.ports.input), editor) else {
        return;
    };
    let Some(d) = active::<G>(&editor) else { return };
    if G::can_connect(&GraphCtxRef { editor: &editor }, d, a, b).is_err() {
        check.reject();
    }
}

/// Verdicts of [`GraphDomain::plan_move_wires`] for the Ctrl+drag running now, by (pin it started from, pin
/// hovered): bevy_noodle asks [`MoveCheck`] for every pin of the direction when the wires are lifted, and again for
/// the pin they are dropped on. The model does not change during a gesture (a drop is planned again when applied).
#[derive(Resource)]
pub struct MoveVerdicts<G> {
    verdicts: HashMap<((String, String), (String, String)), bool>,
    graph: PhantomData<fn() -> G>,
}

impl<G> Default for MoveVerdicts<G> {
    fn default() -> Self {
        MoveVerdicts { verdicts: HashMap::new(), graph: PhantomData }
    }
}

/// Each Ctrl+drag starts with no verdicts: between two the graph, or a function it calls, may have changed.
fn start_move_verdicts<G: GraphDomain>(_: On<Add<MovingEdges>>, mut verdicts: ResMut<MoveVerdicts<G>>) {
    verdicts.verdicts.clear();
}

/// Lifted wires may move to a pin of the same direction exactly where the whole move would succeed (design §5.3): the
/// dry run of [`GraphEdit::MoveWires`] decides alone, over bevy_noodle's built-in verdict (its edits never apply
/// here, [`request_move_wires`]). A refused pin gets no snap and takes no drop.
fn check_move<G: GraphDomain>(
    mut check: On<MoveCheck>,
    keys: Query<&PinKey>,
    editor: Option<Res<Editor>>,
    ec: Option<Res<EditorCanvas<G>>>,
    mut verdicts: ResMut<MoveVerdicts<G>>,
) {
    if ec.as_ref().is_none_or(|c| c.syncing || c.canvas != check.canvas) {
        return;
    }
    let (Ok(from), Ok(to), Some(editor)) = (keys.get(check.from), keys.get(check.to), editor) else { return };
    let (from_pin, to_pin) = ((from.node.clone(), from.pin.clone()), (to.node.clone(), to.pin.clone()));
    let ok = *verdicts.verdicts.entry((from_pin, to_pin)).or_insert_with(|| {
        active::<G>(&editor).is_some_and(|d| G::plan_move_wires(&GraphCtxRef { editor: &editor }, d, from, to).is_ok())
    });
    if ok { check.allow() } else { check.reject() }
}

/// A Ctrl+drag dropped on a pin becomes one [`GraphEdit::MoveWires`] (one undo step, decision 0032). bevy_noodle
/// does not move the edges itself: the model is the source of truth, and the canvas is rebuilt from it (decision
/// 0008).
fn request_move_wires<G: GraphDomain>(
    mut request: On<EditRequested>,
    keys: Query<&PinKey>,
    ec: Option<Res<EditorCanvas<G>>>,
    mut actions: ResMut<Actions>,
) {
    let NoodleEdit::MoveEdges { from, to, .. } = &request.edit else { return };
    if ec.is_none_or(|c| c.canvas != request.canvas) {
        return;
    }
    if let (Ok(from), Ok(to)) = (keys.get(*from), keys.get(*to)) {
        actions.push(GraphEdit::MoveWires {
            node: from.node.clone(),
            pin: from.pin.clone(),
            output: from.output,
            to: (to.node.clone(), to.pin.clone()),
        });
    }
    request.reject();
}

fn sync_from_canvas<G: GraphDomain>(
    mut applied: MessageReader<EditApplied>,
    keys: Query<&PinKey>,
    ec: Option<Res<EditorCanvas<G>>>,
    mut moves: ResMut<PendingMoves<G>>,
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
                    out.push(GraphEdit::Connect {
                        from: (a.node.clone(), a.pin.clone()),
                        to: (b.node.clone(), b.pin.clone()),
                    });
                }
            }
            GraphChange::Disconnected { ports, .. } => {
                if let Ok(b) = keys.get(ports.input) {
                    out.push(GraphEdit::Disconnect { to: (b.node.clone(), b.pin.clone()) });
                }
            }
            GraphChange::Moved { nodes, drag, .. } => {
                if drag.is_none_or(|d| d.is_final) {
                    moves.entities.extend(nodes.iter().copied());
                    moves.quiet_frames = 0;
                }
            }
            // Moves of wires never apply on the canvas: `request_move_wires` turns them into `GraphEdit::MoveWires`.
            GraphChange::Deleted { .. } | GraphChange::EdgesMoved { .. } => {}
        }
    }
    // A connect into a used input reports the replaced edge first; the model connect replaces it anyway.
    out.retain(|a| !matches!(a, GraphEdit::Disconnect { to } if connected_inputs.contains(to)));
    for a in out {
        actions.push(a);
    }
}

/// Commits finished moves (a frame's drag lists the nodes it carries) as one action, a frame later.
fn commit_moves<G: GraphDomain>(
    mut moves: ResMut<PendingMoves<G>>,
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
    actions.push(GraphEdit::Moved { nodes: n, frames: f });
}

fn sync_selection<G: GraphDomain>(
    ec: Option<Res<EditorCanvas<G>>>,
    editor: Res<Editor>,
    selected: Query<(Option<&NodeKey>, Option<&FrameKey>, &GraphOf), With<Selected>>,
    boxes: Query<(&SelectionBox, &CanvasView)>,
    mut actions: ResMut<Actions>,
) {
    let Some(ec) = ec else { return };
    let Some(d) = active::<G>(&editor) else { return };
    if ec.shown.as_ref().map(|s| &s.0) != editor.active_key().as_ref() {
        return;
    }
    let d = G::view(d);
    let mut nodes = BTreeSet::new();
    let mut frames = BTreeSet::new();
    for (n, f, _) in selected.iter().filter(|(.., g)| g.0 == G::VIEW) {
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
            let inside = |f: &&FrameView| {
                let (x, y, w, h) = f.rect;
                area.contains(Vec2::new(x, y)) && area.contains(Vec2::new(x + w, y + h))
            };
            d.frames.iter().filter(|f| frames.contains(&f.id)).filter(inside).map(|f| f.id.clone()).collect()
        }
        Err(_) => frames.intersection(d.enclosed_frames).cloned().collect(),
    };
    let enclosed = enclosed(d.frames.iter().map(|f| (f.id.as_str(), f.nodes.as_slice())), &marked, &nodes);
    if nodes != *d.selection {
        actions.push(GraphEdit::Select(nodes.clone()));
        if !enclosed.is_empty() {
            actions.push(GraphEdit::SelectFrames(enclosed));
        }
    } else if enclosed != *d.enclosed_frames {
        actions.push(GraphEdit::SelectFrames(enclosed));
    }
    let frame = frames.into_iter().next();
    if nodes.is_empty() && frame.as_deref() != d.selected_frame {
        actions.push(GraphEdit::SelectFrame(frame));
    }
}

/// Comment frame colours follow the model without a rebuild (a hue drag recolours live; the canvas is rebuilt
/// only after the gesture).
fn frame_colors<G: GraphDomain>(
    editor: Res<Editor>,
    mut frames: Query<(&FrameKey, &GraphOf, &mut SelectedBorderColor, &mut BackgroundColor, &Children)>,
    mut titles: Query<&mut TextColor>,
) {
    if !editor.is_changed() {
        return;
    }
    let Some(d) = active::<G>(&editor) else { return };
    let d = G::view(d);
    for (key, _, mut border, mut bg, children) in frames.iter_mut().filter(|(_, g, ..)| g.0 == G::VIEW) {
        let Some(f) = d.frames.iter().find(|f| f.id == key.0) else { continue };
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

fn apply_view_requests<G: GraphDomain>(
    mut editor: ResMut<Editor>,
    ec: Option<Res<EditorCanvas<G>>>,
    mut views: Query<(&mut CanvasView, &ComputedNode)>,
    nodes: Query<(&NodePosition, &ComputedNode, &NodeKey, &GraphOf)>,
    graph_views: Option<Res<GraphViews>>,
) {
    let Some(ec) = ec else { return };
    if editor.view_requests.is_empty() {
        return;
    }
    if active::<G>(&editor).is_none() {
        // Another graph domain's canvas carries them out; with no graph active they are dropped.
        let view = editor.tab().map(|t| t.doc().view());
        if !view.is_some_and(|v| graph_views.is_some_and(|g| g.0.contains(&v))) {
            editor.view_requests.clear();
        }
        return;
    }
    if ec.shown.as_ref().map(|s| &s.0) != editor.active_key().as_ref() {
        return;
    }
    let Ok((mut view, canvas)) = views.get_mut(ec.canvas) else { return };
    let size = canvas.size() * canvas.inverse_scale_factor();
    if size.min_element() <= 1.0 {
        return;
    }
    // Back / Forward's camera restore (decision 0041) needs only the canvas size, not the nodes' layout. It lands
    // after `rebuild_canvas`'s per-document restore (the wait for `shown` above).
    if let editor_core::action::ViewRequest::Set { centre, zoom } = editor.view_requests[0] {
        editor.view_requests.remove(0);
        // Not `clamp`, which panics on inverted limits (as in `CanvasView::zoom_around`).
        view.zoom = zoom.max(view.min_zoom).min(view.max_zoom);
        view.pan = size / 2.0 - centre * view.zoom;
        return;
    }
    let req = editor.view_requests[0].clone();
    let Some(d) = active::<G>(&editor) else { return };
    let sel = G::view(d).selection;
    let rects: Vec<Rect> = nodes
        .iter()
        .filter(|(.., g)| g.0 == G::VIEW)
        .filter(|(_, _, k, _)| req == editor_core::action::ViewRequest::FitAll || sel.is_empty() || sel.contains(&k.0))
        .map(|(p, c, ..)| Rect::from_corners(p.0, p.0 + c.size() * c.inverse_scale_factor()))
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
fn keep_center_on_resize<G: GraphDomain>(
    ec: Option<Res<EditorCanvas<G>>>,
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

/// The canvas camera as a navigation history view.
fn nav_view((view, c): (&CanvasView, &ComputedNode)) -> NavView {
    NavView { centre: view_center(view, c), zoom: view.zoom }
}

/// How long the shown graph's camera must rest before [`report_settled_view`] records it (seconds).
pub const NAV_SETTLE_SECS: f32 = 0.3;

/// [`report_settled_view`]'s polling state.
#[derive(Resource)]
struct SettleState<G> {
    /// The document and camera of the previous frame.
    last: Option<(TabKey, NavView)>,
    /// How long `last` has been unchanged.
    still: f32,
    graph: PhantomData<fn() -> G>,
}

impl<G> Default for SettleState<G> {
    fn default() -> Self {
        SettleState { last: None, still: 0.0, graph: PhantomData }
    }
}

/// Records the shown graph's camera in the navigation history once it has rested for [`NAV_SETTLE_SECS`] with no
/// gesture running and no camera request pending (navigation design "Recording camera views", decision 0041).
/// Polling catches every path that moves the camera (pan / zoom gestures, minimap, Fit, the per-document restore)
/// without a fork event. Small moves amend the current entry, large jumps push one (`NavHistory::note_view`). A
/// resting camera is reported once: while it equals the current entry's view there is nothing to record.
fn report_settled_view<G: GraphDomain>(world: &mut World) {
    let Some(canvas) = world.get_resource::<EditorCanvas<G>>().map(|ec| ec.canvas) else { return };
    let shown = world.resource::<EditorCanvas<G>>().shown.as_ref().map(|s| s.0.clone());
    let editor = world.resource::<Editor>();
    let active = shown_key::<G>(editor).map(|(k, _)| k);
    let pending = !editor.view_requests.is_empty();
    let recorded = editor.nav.current().and_then(|e| e.view);
    let Some(id) = active.filter(|a| Some(a) == shown.as_ref()) else { return };
    if pending || !idle::<G>(world) {
        return;
    }
    let Some((view, c)) = world.get::<CanvasView>(canvas).zip(world.get::<ComputedNode>(canvas)) else { return };
    let min_side = (c.size() * c.inverse_scale_factor()).min_element();
    if min_side <= 1.0 {
        return; // not laid out
    }
    let now = nav_view((view, c));
    let dt = world.resource::<Time>().delta_secs();
    let mut state = world.resource_mut::<SettleState<G>>();
    if state.last.as_ref() == Some(&(id.clone(), now)) {
        state.still += dt;
    } else {
        state.last = Some((id.clone(), now));
        state.still = 0.0;
    }
    if state.still < NAV_SETTLE_SECS || recorded == Some(now) {
        return;
    }
    // The history is navigation state only: no redraw or validation keys on it (decision 0041).
    world.resource_mut::<Editor>().bypass_change_detection().nav.note_view(&id, now, min_side);
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
