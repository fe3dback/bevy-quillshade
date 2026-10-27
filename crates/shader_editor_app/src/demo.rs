//! Debug aid: scripted mouse gestures,
//! `SHADER_EDITOR_INPUT=drag|rmb_click|rmb_pan|mmb_pan|slider|color|pin_rmb|node_rmb|node_dblclick|hover|curve_drag|curve_add`
//! `|ctrl_drag_wire|wire_drag|wire_hold|alt_click|box_delete|box_delete_partial`
//! (with `SHADER_EDITOR_SCREENSHOT` and `SHADER_EDITOR_OPEN`; `SHADER_EDITOR_ZOOM=<f32>` sets the canvas zoom
//! first). `slider`/`color` act on the first inline slider / color button on a node, `pin_rmb` right-clicks the
//! first input pin of `SHADER_EDITOR_INPUT_NODE`, `node_rmb` right-clicks the middle of an unselected node (or of
//! `SHADER_EDITOR_INPUT_NODE`) to open its menu, `node_dblclick` double-clicks the middle of that node (opens the
//! file it refers to; the active tab is logged), `hover` rests on the menu_bar's Undo button (its tooltip) or, with
//! `SHADER_EDITOR_INPUT_PIN=<node>.<pin>`, on that pin (its hover halo, design §4.8; hovered pins are logged),
//! `curve_drag` drags the second point of an open curve, `curve_add` double-clicks below it, `texture_zoom` turns the
//! wheel 10 notches over the preview of an open Texture tab (left of its centre), `drop_game_file` drags the Game
//! Assets row of `SHADER_EDITOR_INPUT_FILE` (a path under `assets/`, or a Project tree asset as
//! `shaders/<path>.<kind>.ron`, design §4.11; default: the first game file) onto the middle of
//! `SHADER_EDITOR_INPUT_NODE` (default: the canvas centre) and logs the texture / curve references and function
//! calls of the graph.
//! `ctrl_drag_wire` holds Ctrl and drags from the pin `SHADER_EDITOR_INPUT_PIN=<node>.<pin>` (default: the topmost
//! connected input) onto the pin `SHADER_EDITOR_INPUT_TO=<node>.<pin>` (default: the nearest pin of the same
//! direction on another node), `wire_drag` does the same without Ctrl, `wire_hold` too but keeps the button down
//! (the screenshot shows the dragged wire over the target pin), `alt_click` Alt+clicks the
//! `SHADER_EDITOR_INPUT_PIN` pin; all of them log the wires at both pins before and after (decision 0032).
//! `box_delete` drags a selection box from 20 px outside the first comment frame (or `SHADER_EDITOR_INPUT_FRAME`) to
//! 20 px past its far corner and presses Delete, `box_delete_partial` the same with a box ending in the frame's
//! middle; both log frames, node count and selection before and after (design §4.5). The input goes through
//! the same window messages winit writes,
//! so picking, bevy_noodle and the editor see a real gesture; the outcome is logged.

use bevy::feathers::controls::{FeathersColorInput, FeathersColorWheel, FeathersSlider};
use bevy::input::ButtonState;
use bevy::input::keyboard::{Key, KeyboardInput, NativeKey};
use bevy::input::mouse::{MouseButtonInput, MouseScrollUnit, MouseWheel};
use bevy::input::touch::TouchPhase;
use bevy::prelude::*;
use bevy::ui::ui_transform::UiGlobalTransform;
use bevy::window::{CursorMoved, PrimaryWindow, WindowEvent};
use bevy_noodle::prelude::*;

use crate::canvas::{EditorCanvas, FrameKey, NodeKey, PinHovered, PinKey};
use crate::state::{Editor, PropTarget};
use crate::ui::curve_view::CurveHandle;
use crate::ui::palette::Palette;
use crate::ui::props::PropField;
use crate::ui::texture_view::TextureViewArea;
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

/// Inline editors on canvas nodes: what they edit, where they are, which widget.
type Widgets<'w, 's> = Query<
    'w,
    's,
    (
        &'static PropField,
        &'static UiGlobalTransform,
        &'static ComputedNode,
        Has<FeathersSlider>,
        Has<FeathersColorInput>,
    ),
>;

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
    (handles, previews, rows, frames, hovered): (
        Query<(&CurveHandle, &UiGlobalTransform, &ComputedNode)>,
        Query<(&UiGlobalTransform, &ComputedNode), With<TextureViewArea>>,
        Query<(&TreeRow, &UiGlobalTransform, &ComputedNode)>,
        Query<(&FrameKey, &UiGlobalTransform, &ComputedNode)>,
        Query<&PinKey, With<PinHovered>>,
    ),
    mut events: MessageWriter<WindowEvent>,
    (mut moved, mut wheel): (MessageWriter<CursorMoved>, MessageWriter<MouseWheel>),
    (mut buttons, mut keyboard): (MessageWriter<MouseButtonInput>, MessageWriter<KeyboardInput>),
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
        let mut hovered: Vec<String> = hovered.iter().map(|k| format!("{}.{}", k.node, k.pin)).collect();
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
            "slider" | "color" => {
                let slider = name == "slider";
                let Some((f, t, c, ..)) = widgets.iter().filter(|w| if slider { w.3 } else { w.4 }).min_by(|a, b| {
                    let (a, b) = (a.1.translation, b.1.translation);
                    (a.y, a.x).partial_cmp(&(b.y, b.x)).unwrap()
                }) else {
                    error!("demo {name}: no inline widget on the canvas");
                    return;
                };
                s.target = Some(f.target.clone());
                let at = logical(t, c);
                if slider {
                    // From the left quarter to the right edge of the track.
                    let w = c.size().x * c.inverse_scale_factor();
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
                let click = || [Step::Press(MouseButton::Left), Step::Wait, Step::Release(MouseButton::Left)];
                [Step::Move(logical(t, c))].into_iter().chain(click()).chain([Step::Wait]).chain(click()).collect()
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
                let Some((_, t, c)) = tips.iter().find(|t| t.0.0.starts_with("Undo (")) else {
                    error!("demo {name}: no Undo button");
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
                    let at = at + Vec2::new(0.0, 120.0);
                    let click = || [Step::Press(MouseButton::Left), Step::Wait, Step::Release(MouseButton::Left)];
                    [Step::Move(at)].into_iter().chain(click()).chain([Step::Wait]).chain(click()).collect()
                }
            }
            "texture_zoom" => {
                let Some((t, c)) = previews.iter().next() else {
                    error!("demo {name}: no Texture tab");
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
            "ctrl_drag_wire" | "wire_drag" | "wire_hold" | "alt_click" => {
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
                    let hold = (name == "ctrl_drag_wire").then_some(KeyCode::ControlLeft);
                    // A last move onto the pin itself: picking hovers it before the release.
                    let mut steps = drag(at, MouseButton::Left, logical(tt, tc) - at);
                    steps.insert(steps.len() - 1, Step::Wait);
                    if name == "wire_hold" {
                        steps.pop();
                    }
                    (hold, steps)
                };
                info!("demo {name}: pins {:?} wires {:?}", s.pins, wires_at(&editor, &s.pins));
                let mut all: Vec<Step> = hold.map(|k| Step::Key(k, true)).into_iter().collect();
                all.extend(steps);
                all.extend(hold.map(|k| Step::Key(k, false)));
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
                keyboard.write(KeyboardInput {
                    key_code,
                    logical_key: Key::Unidentified(NativeKey::Unidentified),
                    state: if down { ButtonState::Pressed } else { ButtonState::Released },
                    text: None,
                    repeat: false,
                    window,
                });
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
            None if s.frame == START + 40 && name == "node_dblclick" => {
                let tab = editor.active.and_then(|i| editor.tabs[i].owner()).map(shader_project::AssetId::source_rel);
                info!("demo node_dblclick: active tab {}", tab.unwrap_or_default());
            }
            None if s.frame == START + 40 && !s.pins.is_empty() => {
                let rev = editor.doc().map(|d| d.revision);
                let last = editor.messages.last().map(|m| m.text.as_str());
                info!(
                    "demo {name}: rev {rev:?} wires {:?} palette {} last message {last:?}",
                    wires_at(&editor, &s.pins),
                    palette.open
                );
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
