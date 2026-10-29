//! Debug aid: scripted mouse gestures,
//! `SHADER_EDITOR_INPUT=drag|rmb_click|rmb_pan|mmb_pan|slider|color|pin_rmb|node_rmb|node_dblclick|frame_dblclick`
//! `|hover|curve_drag|curve_add|texture_zoom|drop_game_file`
//! `|ctrl_drag_wire|ctrl_wire_hold|wire_drag|wire_hold|alt_click|box_delete|box_delete_partial|type_field`
//! `|menu_click|menu_close|tab[:N]`
//! (with `SHADER_EDITOR_SCREENSHOT` and `SHADER_EDITOR_OPEN`; `SHADER_EDITOR_ZOOM=<f32>` sets the canvas zoom
//! first). `slider` drags the topmost number input with a soft limit (an inline ranged field on a node wholly inside
//! the visible canvas, or the Hue of a selected comment in the right sidebar) across most of its on-screen width and
//! logs the field's value and its node's position, `color` clicks the first inline color button on
//! a node, `pin_rmb` right-clicks the first input pin of `SHADER_EDITOR_INPUT_NODE`, `node_rmb` right-clicks the
//! middle of an unselected node (or of `SHADER_EDITOR_INPUT_NODE`) to open its menu, `node_dblclick` double-clicks
//! the middle of that node (opens the file it refers to) or, with `SHADER_EDITOR_INPUT_AT=header`, its header (the
//! rename / comment dialog, design §5.5; the active tab and the dialog are logged), `frame_dblclick` double-clicks
//! the title strip of the first comment frame (or `SHADER_EDITOR_INPUT_FRAME`; the dialog is logged), `hover` rests
//! on the menu_bar's Back button (its tooltip), on the widget whose tooltip starts with `SHADER_EDITOR_INPUT_TIP`
//! or, with `SHADER_EDITOR_INPUT_PIN=<node>.<pin>`, on that pin (its hover look, design §4.8; hovered pins are logged),
//! `curve_drag` drags the second point of an open curve, `curve_add` double-clicks below it, `texture_zoom` turns
//! the wheel 10 notches over the preview of an open image or Texture tab (left of its centre),
//! `drop_game_file` drags the Game Assets row of `SHADER_EDITOR_INPUT_FILE` (a path under `assets/`, or a Project
//! tree asset as `shaders/<path>.<kind>.ron`, design §4.11; default: the first game file) onto the middle of
//! `SHADER_EDITOR_INPUT_NODE` (default: the canvas centre) and logs the texture / curve references and function
//! calls of the graph.
//! `ctrl_drag_wire` holds Ctrl and drags from the pin `SHADER_EDITOR_INPUT_PIN=<node>.<pin>` (default: the topmost
//! connected input) onto the pin `SHADER_EDITOR_INPUT_TO=<node>.<pin>` (default: the nearest pin of the same
//! direction on another node), `wire_drag` does the same without Ctrl, `wire_hold` too but keeps the button down
//! (the screenshot shows the dragged wire over the target pin), `ctrl_wire_hold` is `ctrl_drag_wire` keeping Ctrl
//! and the button down (the screenshot shows the lifted wires mid-gesture, design §5.3; the lifted wires and the pin
//! they snap to are logged); with `SHADER_EDITOR_INPUT_AT=<dx>,<dy>` these drags end that many logical px from the
//! pin dragged from instead of on the target pin, with `SHADER_EDITOR_INPUT_AT=to:<dx>,<dy>` that far from the
//! target pin. `alt_click` Alt+clicks the `SHADER_EDITOR_INPUT_PIN` pin; all of
//! them log the wires at both pins before and after (decision 0032).
//! `box_delete` drags a selection box from 20 px outside the first comment frame (or `SHADER_EDITOR_INPUT_FRAME`) to
//! 20 px past its far corner and presses Delete, `box_delete_partial` the same with a box ending in the frame's
//! middle; both log frames, node count and selection before and after (design §4.5). `type_field` clicks the
//! right sidebar's Description field of an open graph, types `ab` and logs the focus and the field's text.
//! `menu_click` clicks the menu_bar's menu `SHADER_EDITOR_INPUT_MENU=<name>` (default `file`), `menu_close` then
//! clicks the middle of the canvas (on the menu's barrier); both log each main menu's open and active (`Pressed`)
//! state (design §1). `tab:N` presses Tab N times (default 1, at most 30) and logs after each press the focused
//! entity, its tooltip and action and whether a `Display::None` ancestor hides it; the screenshot shows the focus
//! ring (design §2). The input
//! goes through the same window messages winit writes, so picking, bevy_noodle and the editor see a real gesture; the
//! outcome is logged.

use bevy::feathers::controls::{FeathersColorInput, FeathersColorWheel, SoftLimit};
use bevy::input::ButtonState;
use bevy::input::keyboard::{Key, KeyboardInput, NativeKey};
use bevy::input::mouse::{MouseButtonInput, MouseScrollUnit, MouseWheel};
use bevy::input::touch::TouchPhase;
use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::ui::ui_transform::UiGlobalTransform;
use bevy::ui_widgets::MenuButton;
use bevy::window::{CursorMoved, PrimaryWindow, WindowEvent};
use bevy_noodle::prelude::*;

use crate::canvas::{EditorCanvas, FrameKey, FrameTitleStrip, NodeHeader, NodeKey, PinKey};
use crate::state::{Editor, PropTarget};
use crate::ui::MainMenu;
use crate::ui::curve_view::CurveHandle;
use crate::ui::image_preview::PreviewArea;
use crate::ui::palette::Palette;
use crate::ui::props::PropField;
use crate::ui::tooltip::Tooltip;
use crate::ui::tree::{TreeItem, TreeRow};

/// First frame of the gesture (the asset opens on frame 2, the screenshot is taken on frame 240).
const START: u32 = 160;

enum Step {
    Move(Vec2),
    Press(MouseButton),
    Release(MouseButton),
    /// A key down (`true`) or up.
    Key(KeyCode, bool),
    /// A key typing this character (down and up).
    Type(KeyCode, char),
    /// Wheel notches (positive: up / zoom in).
    Wheel(f32),
    Wait,
}

#[derive(Default)]
pub struct Script {
    frame: u32,
    steps: Vec<Step>,
    node: Option<String>,
    /// Inline editor the gesture acts on (its value is logged).
    target: Option<PropTarget>,
    /// Pins whose wires the wire gestures log.
    pins: Vec<PinKey>,
}

/// `SHADER_EDITOR_INPUT_PIN` / `_TO`: `<node>.<pin>`.
fn pin_env(var: &str) -> Option<(String, String)> {
    let v = std::env::var(var).ok()?;
    let (node, pin) = v.split_once('.')?;
    Some((node.into(), pin.into()))
}

/// The wires at `pins`, as `from.pin -> to.pin`.
fn wires_at(editor: &Editor, pins: &[PinKey]) -> Vec<String> {
    let Some(d) = editor.doc() else { return Vec::new() };
    let at = |k: &PinKey, end: &(String, String)| k.node == end.0 && k.pin == end.1;
    d.asset
        .edges
        .iter()
        .filter(|e| pins.iter().any(|k| if k.output { at(k, &e.from) } else { at(k, &e.to) }))
        .map(|e| format!("{}.{} -> {}.{}", e.from.0, e.from.1, e.to.0, e.to.1))
        .collect()
}

/// Comment frames, node count, node selection and enclosed frames of the active graph.
fn frames_log(editor: &Editor) -> String {
    let Some(d) = editor.doc() else { return String::new() };
    let frames: Vec<&str> = d.asset.comments.iter().map(|f| f.id.as_str()).collect();
    format!(
        "rev {} frames {frames:?} nodes {} selection {:?} enclosed {:?}",
        d.revision,
        d.asset.nodes.len(),
        d.selection,
        d.enclosed_frames
    )
}

/// Property widgets (inline editors on canvas nodes and the right sidebar's): what they edit, where they are, which
/// widget.
type Widgets<'w, 's> = Query<
    'w,
    's,
    (
        &'static PropField,
        &'static UiGlobalTransform,
        &'static ComputedNode,
        Has<SoftLimit>,
        Has<FeathersColorInput>,
        Entity,
    ),
>;

/// The logical on-screen size of a UI node: layout size times the scale of its transform (the canvas zoom for
/// anything on the canvas).
fn screen_size(t: &UiGlobalTransform, c: &ComputedNode) -> Vec2 {
    c.size() * t.to_scale_angle_translation().0 * c.inverse_scale_factor()
}

/// Two left clicks at `at`.
fn double_click(at: Vec2) -> Vec<Step> {
    let click = || [Step::Press(MouseButton::Left), Step::Wait, Step::Release(MouseButton::Left)];
    [Step::Move(at)].into_iter().chain(click()).chain([Step::Wait]).chain(click()).collect()
}

fn drag(from: Vec2, button: MouseButton, by: Vec2) -> Vec<Step> {
    let mut steps = vec![Step::Move(from), Step::Press(button)];
    steps.extend((1..=10).map(|i| Step::Move(from + by * i as f32 / 10.0)));
    steps.push(Step::Release(button));
    steps
}

#[allow(clippy::too_many_arguments)]
pub fn scripted_input(
    mut s: Local<Script>,
    editor: Res<Editor>,
    ec: Option<Res<EditorCanvas>>,
    palette: Res<Palette>,
    window: Query<Entity, With<PrimaryWindow>>,
    nodes: Query<(&NodeKey, &NodePosition, &UiGlobalTransform, &ComputedNode)>,
    mut canvases: Query<(&mut CanvasView, &UiGlobalTransform, &ComputedNode)>,
    widgets: Widgets,
    ports: Query<(&PinKey, &UiGlobalTransform, &ComputedNode)>,
    wheels: Query<(&UiGlobalTransform, &ComputedNode), With<FeathersColorWheel>>,
    tips: Query<(&Tooltip, &UiGlobalTransform, &ComputedNode)>,
    (handles, previews, rows, frames, hovered, headers, strips): (
        Query<(&CurveHandle, &UiGlobalTransform, &ComputedNode)>,
        Query<(&PreviewArea, &UiGlobalTransform, &ComputedNode)>,
        Query<(&TreeRow, &UiGlobalTransform, &ComputedNode)>,
        Query<(&FrameKey, &UiGlobalTransform, &ComputedNode)>,
        Query<(&PinKey, &Hovered)>,
        Query<(&ChildOf, &UiGlobalTransform, &ComputedNode), With<NodeHeader>>,
        Query<(&ChildOf, &UiGlobalTransform, &ComputedNode), With<FrameTitleStrip>>,
    ),
    mut events: MessageWriter<WindowEvent>,
    (mut moved, mut wheel): (MessageWriter<CursorMoved>, MessageWriter<MouseWheel>),
    (mut buttons, mut keyboard): (MessageWriter<MouseButtonInput>, MessageWriter<KeyboardInput>),
    (focus, texts, lifts, wires, parents, menus, menu_parts, layout, acts, ring): (
        Res<bevy::input_focus::InputFocus>,
        Query<&bevy::text::EditableText>,
        Query<(&MovingEdges, &WireTarget)>,
        Query<&WireTarget, With<PendingWire>>,
        Query<&ChildOf>,
        Query<(&MainMenu, &Children, &UiGlobalTransform, &ComputedNode)>,
        Query<(Option<&Visibility>, Has<bevy::ui::Pressed>, Has<MenuButton>)>,
        Query<&Node>,
        Query<&crate::ui::OnActivate>,
        Res<bevy::input_focus::InputFocusVisible>,
    ),
) {
    let Ok(name) = std::env::var("SHADER_EDITOR_INPUT") else { return };
    let (Ok(window), Some(ec)) = (window.single(), ec) else { return };
    s.frame += 1;
    if s.frame == START - 20
        && let Some(zoom) = std::env::var("SHADER_EDITOR_ZOOM").ok().and_then(|z| z.parse().ok())
        && let Ok((mut view, ..)) = canvases.get_mut(ec.canvas)
    {
        view.zoom = zoom;
    }
    let Ok((view, ct, cc)) = canvases.get(ec.canvas) else { return };
    let logical = |t: &UiGlobalTransform, c: &ComputedNode| t.translation * c.inverse_scale_factor();
    let report = |s: &Script| {
        let d = editor.doc();
        let model = s.node.as_ref().and_then(|n| d?.asset.nodes.get(n)).map(|n| n.pos);
        let shown = nodes.iter().find(|n| Some(&n.0.0) == s.node.as_ref()).map(|n| n.1.0);
        let selection = d.map(|d| d.selection.clone()).unwrap_or_default();
        let value = match &s.target {
            Some(PropTarget::Input { node, pin }) => d.and_then(|d| d.asset.nodes.get(node)?.inputs.get(pin).cloned()),
            Some(PropTarget::Param { node, key }) => d.and_then(|d| d.asset.nodes.get(node)?.params.get(key).cloned()),
            Some(t @ PropTarget::FrameHue(_)) => editor.current_value(t),
            _ => None,
        };
        let focus_or_popup = palette.open;
        let rev = d.map(|d| d.revision);
        let mut hovered: Vec<String> =
            hovered.iter().filter(|(_, h)| h.0).map(|(k, _)| format!("{}.{}", k.node, k.pin)).collect();
        hovered.sort();
        if let Some(c) = editor.curve_doc() {
            info!("demo {name} f{} curve {:?} selection {:?}", s.frame, c.curve.points, c.selection);
            return;
        }
        info!(
            "demo {name} f{} rev {rev:?} shown {:?}: node {:?} model {model:?} canvas {shown:?} selection {selection:?} pan {} palette {focus_or_popup} target {:?} = {value:?} hovered pins {hovered:?}",
            s.frame,
            ec.shown.as_ref().map(|s| s.1),
            s.node,
            view.pan,
            s.target,
        );
    };
    if s.frame == START {
        let canvas_min = logical(ct, cc) - cc.size() * cc.inverse_scale_factor() / 2.0;
        let wanted = std::env::var("SHADER_EDITOR_INPUT_NODE").ok();
        let unselected = nodes
            .iter()
            .filter(|n| editor.doc().is_some_and(|d| !d.selection.contains(&n.0.0)))
            .filter(|n| wanted.as_ref().is_none_or(|w| *w == n.0.0))
            .min_by_key(|n| &n.0.0);
        s.steps = match name.as_str() {
            "drag" => {
                let Some((key, _, t, c)) = unselected else { return };
                s.node = Some(key.0.clone());
                drag(logical(t, c), MouseButton::Left, Vec2::new(120.0, 60.0))
            }
            "type_field" => {
                let Some((f, t, c, ..)) = widgets.iter().find(|w| w.0.target == PropTarget::Description) else {
                    error!("demo {name}: no Description field (open a graph)");
                    return;
                };
                s.target = Some(f.target.clone());
                let at = logical(t, c);
                vec![
                    Step::Move(at),
                    Step::Press(MouseButton::Left),
                    Step::Wait,
                    Step::Release(MouseButton::Left),
                    Step::Wait,
                    Step::Wait,
                    Step::Type(KeyCode::KeyA, 'a'),
                    Step::Wait,
                    Step::Type(KeyCode::KeyB, 'b'),
                ]
            }
            "slider" | "color" => {
                let slider = name == "slider";
                // A widget on the canvas must lie wholly inside the canvas' rect, or the press misses it.
                let canvas_rect = Rect::from_center_size(logical(ct, cc), cc.size() * cc.inverse_scale_factor());
                let reachable = |(_, t, c, _, _, e): &(&PropField, &UiGlobalTransform, &ComputedNode, _, _, _)| {
                    let rect = Rect::from_center_size(logical(t, c), screen_size(t, c));
                    !parents.iter_ancestors(*e).any(|a| a == ec.canvas)
                        || canvas_rect.contains(rect.min) && canvas_rect.contains(rect.max)
                };
                let Some((f, t, c, ..)) =
                    widgets.iter().filter(|w| if slider { w.3 } else { w.4 }).filter(reachable).min_by(|a, b| {
                        let (a, b) = (a.1.translation, b.1.translation);
                        (a.y, a.x).partial_cmp(&(b.y, b.x)).unwrap()
                    })
                else {
                    error!("demo {name}: no widget on screen (pan or zoom with SHADER_EDITOR_ZOOM)");
                    return;
                };
                s.target = Some(f.target.clone());
                // The node owning an inline field: the report logs its position (a scrub must not move it).
                s.node = match &f.target {
                    PropTarget::Input { node, .. } | PropTarget::Param { node, .. } => Some(node.clone()),
                    _ => None,
                };
                let at = logical(t, c);
                if slider {
                    // From the left quarter of the field to its right edge (a press without a move would type);
                    // on screen, so scaled by the canvas zoom for an inline field.
                    let w = screen_size(t, c).x;
                    info!("demo {name}: field at {at} {w} px wide");
                    drag(at - Vec2::new(w / 4.0, 0.0), MouseButton::Left, Vec2::new(w * 0.7, 0.0))
                } else {
                    vec![Step::Move(at), Step::Press(MouseButton::Left), Step::Wait, Step::Release(MouseButton::Left)]
                }
            }
            "pin_rmb" => {
                let wanted = std::env::var("SHADER_EDITOR_INPUT_NODE").ok();
                let Some((_, t, c)) = ports
                    .iter()
                    .filter(|p| !p.0.output && wanted.as_ref().is_none_or(|w| *w == p.0.node))
                    .min_by(|a, b| a.1.translation.y.partial_cmp(&b.1.translation.y).unwrap())
                else {
                    error!("demo {name}: no input pin");
                    return;
                };
                let at = logical(t, c);
                vec![Step::Move(at), Step::Press(MouseButton::Right), Step::Wait, Step::Release(MouseButton::Right)]
            }
            "node_rmb" => {
                let Some((key, _, t, c)) = unselected else { return };
                s.node = Some(key.0.clone());
                let at = logical(t, c);
                vec![Step::Move(at), Step::Press(MouseButton::Right), Step::Wait, Step::Release(MouseButton::Right)]
            }
            "node_dblclick" => {
                let Some((key, _, t, c)) = unselected else { return };
                s.node = Some(key.0.clone());
                let at = match std::env::var("SHADER_EDITOR_INPUT_AT").as_deref() {
                    Ok("header") => {
                        let of_node = |p: &ChildOf| nodes.get(p.parent()).is_ok_and(|n| n.0.0 == key.0);
                        let Some((_, t, c)) = headers.iter().find(|h| of_node(h.0)) else {
                            error!("demo {name}: node {} has no header", key.0);
                            return;
                        };
                        logical(t, c)
                    }
                    _ => logical(t, c),
                };
                double_click(at)
            }
            "frame_dblclick" => {
                let wanted = std::env::var("SHADER_EDITOR_INPUT_FRAME").ok();
                let of_frame =
                    |p: &ChildOf| frames.get(p.parent()).is_ok_and(|f| wanted.as_ref().is_none_or(|w| *w == f.0.0));
                let strip = strips
                    .iter()
                    .filter(|s| of_frame(s.0))
                    .min_by_key(|s| frames.get(s.0.parent()).map(|f| f.0.0.clone()).unwrap_or_default());
                let Some((_, t, c)) = strip else {
                    error!("demo {name}: no comment frame {wanted:?}");
                    return;
                };
                double_click(logical(t, c))
            }
            "rmb_click" => vec![Step::Move(canvas_min + 20.0), Step::Press(MouseButton::Right), Step::Wait, Step::Wait]
                .into_iter()
                .chain([Step::Release(MouseButton::Right)])
                .collect(),
            "hover" if let Some((n, pin)) = pin_env("SHADER_EDITOR_INPUT_PIN") => {
                let Some((_, t, c)) = ports.iter().find(|p| p.0.node == n && p.0.pin == pin) else {
                    error!("demo {name}: no pin {n}.{pin}");
                    return;
                };
                vec![Step::Move(logical(t, c))]
            }
            "hover" => {
                let tip = std::env::var("SHADER_EDITOR_INPUT_TIP").unwrap_or_else(|_| "Back (".into());
                let Some((_, t, c)) = tips.iter().find(|t| t.0.0.starts_with(&tip)) else {
                    error!("demo {name}: no widget with the tooltip {tip:?}");
                    return;
                };
                vec![Step::Move(logical(t, c))]
            }
            "curve_drag" | "curve_add" => {
                let Some((_, t, c)) = handles.iter().find(|h| h.0.index() == 1) else {
                    error!("demo {name}: no curve point");
                    return;
                };
                let at = logical(t, c);
                if name == "curve_drag" {
                    drag(at, MouseButton::Left, Vec2::new(80.0, 60.0))
                } else {
                    double_click(at + Vec2::new(0.0, 120.0))
                }
            }
            "texture_zoom" => {
                let kind = editor.tab().map(|t| t.doc().view());
                let Some((_, t, c)) = previews.iter().find(|(a, ..)| Some(a.0) == kind) else {
                    error!("demo {name}: no image or Texture tab");
                    return;
                };
                let at = logical(t, c) - Vec2::new(60.0, 0.0);
                [Step::Move(at)].into_iter().chain((0..10).map(|_| Step::Wheel(1.0))).collect()
            }
            "drop_game_file" => {
                let file = std::env::var("SHADER_EDITOR_INPUT_FILE").ok();
                let row = rows.iter().find(|r| match &r.0.item {
                    TreeItem::GameFile(p) => file.as_ref().is_none_or(|f| f == p),
                    TreeItem::Asset(id) => file.as_ref().is_some_and(|f| *f == id.source_rel()),
                    _ => false,
                });
                let Some((row, t, c)) = row else {
                    error!("demo {name}: no Game Assets row {file:?}");
                    return;
                };
                let target = wanted.as_ref().and_then(|w| nodes.iter().find(|n| n.0.0 == *w));
                s.node = target.map(|n| n.0.0.clone());
                let to = target.map_or(logical(ct, cc), |(_, _, t, c)| logical(t, c));
                let from = logical(t, c);
                info!("demo {name}: dragging {:?} from {from} to {to}", row.item);
                drag(from, MouseButton::Left, to - from)
            }
            "ctrl_drag_wire" | "ctrl_wire_hold" | "wire_drag" | "wire_hold" | "alt_click" => {
                let d = editor.doc();
                let connected = |k: &PinKey| {
                    d.is_some_and(|d| d.asset.edges.iter().any(|e| e.to == (k.node.clone(), k.pin.clone())))
                };
                let wanted = pin_env("SHADER_EDITOR_INPUT_PIN");
                let from = ports
                    .iter()
                    .filter(|p| match &wanted {
                        Some((n, pin)) => p.0.node == *n && p.0.pin == *pin,
                        None => !p.0.output && connected(p.0),
                    })
                    .min_by(|a, b| a.1.translation.y.partial_cmp(&b.1.translation.y).unwrap());
                let Some((from, ft, fc)) = from else {
                    error!("demo {name}: no pin {wanted:?}");
                    return;
                };
                let at = logical(ft, fc);
                let (hold, steps) = if name == "alt_click" {
                    s.pins = vec![from.clone()];
                    let click = [Step::Press(MouseButton::Left), Step::Wait, Step::Release(MouseButton::Left)];
                    (Some(KeyCode::AltLeft), [Step::Move(at)].into_iter().chain(click).collect())
                } else {
                    let wanted = pin_env("SHADER_EDITOR_INPUT_TO");
                    let to = ports
                        .iter()
                        .filter(|p| match &wanted {
                            Some((n, pin)) => p.0.node == *n && p.0.pin == *pin,
                            None => p.0.output == from.output && p.0.node != from.node,
                        })
                        .min_by(|a, b| {
                            let d = |t: &UiGlobalTransform, c: &ComputedNode| logical(t, c).distance(at);
                            d(a.1, a.2).partial_cmp(&d(b.1, b.2)).unwrap()
                        });
                    let Some((to, tt, tc)) = to else {
                        error!("demo {name}: no target pin {wanted:?}");
                        return;
                    };
                    s.pins = vec![from.clone(), to.clone()];
                    let hold = name.starts_with("ctrl_").then_some(KeyCode::ControlLeft);
                    // `SHADER_EDITOR_INPUT_AT=<dx>,<dy>`: end that far from the pin dragged from (empty canvas);
                    // `to:<dx>,<dy>`: that far from the target pin (its snap radius, design §6.3).
                    let at_env = std::env::var("SHADER_EDITOR_INPUT_AT").ok();
                    let (base, offset) = match at_env.as_deref().map(|v| v.strip_prefix("to:").ok_or(v)) {
                        Some(Ok(v)) => (logical(tt, tc), Some(v)),
                        Some(Err(v)) => (at, Some(v)),
                        None => (at, None),
                    };
                    let offset = offset.and_then(|v| {
                        let (x, y) = v.split_once(',')?;
                        Some(Vec2::new(x.trim().parse().ok()?, y.trim().parse().ok()?))
                    });
                    let end = offset.map_or(logical(tt, tc), |o| base + o);
                    // A last move onto the pin itself: picking hovers it before the release.
                    let mut steps = drag(at, MouseButton::Left, end - at);
                    steps.insert(steps.len() - 1, Step::Wait);
                    if name.ends_with("_hold") {
                        steps.pop();
                    }
                    (hold, steps)
                };
                info!("demo {name}: pins {:?} wires {:?}", s.pins, wires_at(&editor, &s.pins));
                let mut all: Vec<Step> = hold.map(|k| Step::Key(k, true)).into_iter().collect();
                all.extend(steps);
                // `ctrl_wire_hold` keeps Ctrl down with the button, for the mid-gesture screenshot.
                all.extend(hold.filter(|_| !name.ends_with("_hold")).map(|k| Step::Key(k, false)));
                all
            }
            "box_delete" | "box_delete_partial" => {
                let wanted = std::env::var("SHADER_EDITOR_INPUT_FRAME").ok();
                let frame =
                    frames.iter().filter(|f| wanted.as_ref().is_none_or(|w| *w == f.0.0)).min_by_key(|f| &f.0.0);
                let Some((_, t, c)) = frame else {
                    error!("demo {name}: no comment frame {wanted:?}");
                    return;
                };
                // Layout size is in graph units: scaled by the canvas zoom on screen.
                let half = c.size() * c.inverse_scale_factor() * view.zoom / 2.0;
                let (centre, margin) = (logical(t, c), Vec2::splat(20.0));
                let from = centre - half - margin;
                let to = if name == "box_delete" { centre + half + margin } else { centre };
                info!("demo {name}: box {from} to {to}, {}", frames_log(&editor));
                let mut steps = drag(from, MouseButton::Left, to - from);
                steps.extend([Step::Wait, Step::Wait, Step::Key(KeyCode::Delete, true), Step::Wait]);
                steps.push(Step::Key(KeyCode::Delete, false));
                steps
            }
            "menu_click" | "menu_close" => {
                let wanted = std::env::var("SHADER_EDITOR_INPUT_MENU").unwrap_or_else(|_| "file".into());
                let Some((.., t, c)) = menus.iter().find(|m| m.0.label().eq_ignore_ascii_case(&wanted)) else {
                    error!("demo {name}: no main menu {wanted:?}");
                    return;
                };
                let click =
                    |at| [Step::Move(at), Step::Press(MouseButton::Left), Step::Wait, Step::Release(MouseButton::Left)];
                let mut steps: Vec<Step> =
                    click(logical(t, c)).into_iter().chain((0..10).map(|_| Step::Wait)).collect();
                if name == "menu_close" {
                    steps.extend(click(logical(ct, cc)));
                }
                steps
            }
            tab if tab == "tab" || tab.starts_with("tab:") => {
                let n = tab.strip_prefix("tab:").and_then(|n| n.parse().ok()).unwrap_or(1usize).min(30);
                (0..n).flat_map(|_| [Step::Key(KeyCode::Tab, true), Step::Key(KeyCode::Tab, false)]).collect()
            }
            "rmb_pan" => drag(canvas_min + 20.0, MouseButton::Right, Vec2::new(150.0, 80.0)),
            "mmb_pan" => drag(canvas_min + 20.0, MouseButton::Middle, Vec2::new(150.0, 80.0)),
            other => {
                error!("unknown SHADER_EDITOR_INPUT={other}");
                Vec::new()
            }
        };
        s.steps.reverse();
        report(&s);
    }
    if s.frame > START && std::env::var("SHADER_EDITOR_INPUT_TRACE").is_ok() && s.frame < START + 20 {
        report(&s);
    }
    // `color`: once the picker is open, drag across its wheel.
    if name == "color"
        && s.frame == START + 10
        && let Some((t, c)) = wheels.iter().next()
    {
        let at = logical(t, c);
        s.steps = drag(at + Vec2::new(-30.0, 0.0), MouseButton::Left, Vec2::new(40.0, 25.0));
        s.steps.reverse();
    }
    if s.frame > START {
        match s.steps.pop() {
            Some(Step::Move(position)) => {
                let e = CursorMoved { window, position, delta: None };
                moved.write(e.clone());
                events.write(e.into());
            }
            Some(Step::Press(button)) => {
                let e = MouseButtonInput { button, state: ButtonState::Pressed, window };
                buttons.write(e);
                events.write(e.into());
            }
            Some(Step::Release(button)) => {
                let e = MouseButtonInput { button, state: ButtonState::Released, window };
                buttons.write(e);
                events.write(e.into());
            }
            Some(Step::Key(key_code, down)) => {
                if key_code == KeyCode::Tab && !down {
                    // The focus the press (the frame before) moved.
                    let f = focus.get();
                    let tip = f.and_then(|f| tips.get(f).ok()).map(|t| t.0.0.clone());
                    let action = f.and_then(|f| acts.get(f).ok()).map(|a| a.0.clone());
                    let hidden = f.is_some_and(|f| {
                        std::iter::once(f)
                            .chain(parents.iter_ancestors(f))
                            .any(|a| layout.get(a).is_ok_and(|n| n.display == Display::None))
                    });
                    info!(
                        "demo {name}: focus {f:?} tip {tip:?} action {action:?} display-hidden {hidden} ring {}",
                        ring.0
                    );
                }
                keyboard.write(KeyboardInput {
                    key_code,
                    logical_key: Key::Unidentified(NativeKey::Unidentified),
                    state: if down { ButtonState::Pressed } else { ButtonState::Released },
                    text: None,
                    repeat: false,
                    window,
                });
            }
            Some(Step::Type(key_code, c)) => {
                for state in [ButtonState::Pressed, ButtonState::Released] {
                    let text = (state == ButtonState::Pressed).then(|| c.to_string().into());
                    let logical_key = Key::Character(c.to_string().into());
                    keyboard.write(KeyboardInput { key_code, logical_key, state, text, repeat: false, window });
                }
            }
            Some(Step::Wheel(y)) => {
                let e = MouseWheel { unit: MouseScrollUnit::Line, x: 0.0, y, window, phase: TouchPhase::Moved };
                wheel.write(e);
                events.write(e.into());
            }
            Some(Step::Wait) => {}
            None if s.frame == START + 40 && name.starts_with("box_delete") => {
                info!("demo {name}: {}", frames_log(&editor));
            }
            None if s.frame == START + 40 && name == "type_field" => {
                let focused = focus.get();
                let text = focused.and_then(|f| texts.get(f).ok()).map(|t| t.value().to_string());
                let field = widgets.iter().find(|w| w.0.target == PropTarget::Description).is_some();
                info!("demo type_field: focus {focused:?} field shown {field} focused text {text:?}");
            }
            None if s.frame == START + 40 && (name == "node_dblclick" || name == "frame_dblclick") => {
                let tab = editor.active.and_then(|i| editor.tabs[i].owner()).map(shader_project::AssetId::source_rel);
                info!("demo {name}: active tab {} dialog {:?}", tab.unwrap_or_default(), editor.dialog);
            }
            None if s.frame == START + 40 && !s.pins.is_empty() => {
                let rev = editor.doc().map(|d| d.revision);
                let last = editor.messages.last().map(|m| m.text.as_str());
                info!(
                    "demo {name}: rev {rev:?} wires {:?} palette {} last message {last:?}",
                    wires_at(&editor, &s.pins),
                    palette.open
                );
                // Mid-gesture (`*_hold`): what is dragged, lifted wires or a new one.
                let pin = |e: Entity| ports.get(e).ok().map(|p| format!("{}.{}", p.0.node, p.0.pin));
                for (lifted, target) in &lifts {
                    let (from, to) = (pin(lifted.from), target.0.and_then(pin));
                    info!("demo {name}: lifted {} wires from {from:?}, snapped to {to:?}", lifted.edges.len());
                }
                let snapped: Vec<_> = wires.iter().map(|t| t.0.and_then(pin)).collect();
                info!("demo {name}: dragged new wires {}, snapped to {snapped:?}", snapped.len());
            }
            None if s.frame == START + 40 && name.starts_with("menu_") => {
                for (menu, children, ..) in &menus {
                    // The popup's visibility and the button's `Pressed`.
                    let (mut open, mut active) = (false, false);
                    for (visibility, pressed, button) in children.iter().filter_map(|c| menu_parts.get(c).ok()) {
                        open |= !button && visibility == Some(&Visibility::Visible);
                        active |= button && pressed;
                    }
                    info!("demo {name}: menu {} open {open} active {active}", menu.label());
                }
            }
            None if s.frame == START + 40 && name == "drop_game_file" => {
                let Some(d) = editor.doc() else { return };
                for (id, n) in &d.asset.nodes {
                    let tex = ["texture", "default", "curve"].iter().find_map(|k| n.params.get(*k));
                    let kinds = ["Texture", "SampleCurve", shader_graph_core::registry::FUNCTION_PREFIX];
                    if kinds.iter().any(|k| n.kind.contains(k)) {
                        info!("demo drop_game_file: rev {} node {id} {} at {:?} = {tex:?}", d.revision, n.kind, n.pos);
                    }
                }
                let last = editor.messages.last().map(|m| m.text.as_str());
                info!("demo drop_game_file: selection {:?} last message {last:?}", d.selection);
            }
            None if s.frame == START + 40 => report(&s),
            None => {}
        }
    }
}
