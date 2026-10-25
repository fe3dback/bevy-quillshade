//! Debug aid: scripted mouse gestures,
//! `SHADER_EDITOR_INPUT=drag|rmb_click|rmb_pan|mmb_pan|slider|color|pin_rmb|hover|curve_drag|curve_add`
//! (with `SHADER_EDITOR_SCREENSHOT` and `SHADER_EDITOR_OPEN`; `SHADER_EDITOR_ZOOM=<f32>` sets the canvas zoom
//! first). `slider`/`color` act on the first inline slider / color button on a node, `pin_rmb` right-clicks the
//! first input pin of `SHADER_EDITOR_INPUT_NODE`, `hover` rests on the toolbar's Undo button (its tooltip),
//! `curve_drag` drags the second point of an open curve, `curve_add` double-clicks below it. The input goes through
//! the same window messages winit writes,
//! so picking, bevy_noodle and the editor see a real gesture; the outcome is logged.

use bevy::feathers::controls::{FeathersColorInput, FeathersColorWheel, FeathersSlider};
use bevy::input::ButtonState;
use bevy::input::mouse::MouseButtonInput;
use bevy::prelude::*;
use bevy::ui::ui_transform::UiGlobalTransform;
use bevy::window::{CursorMoved, PrimaryWindow, WindowEvent};
use bevy_noodle::prelude::*;

use crate::canvas::{EditorCanvas, NodeKey, PinKey};
use crate::state::{Editor, PropTarget};
use crate::ui::curve_view::CurveHandle;
use crate::ui::palette::Palette;
use crate::ui::props::PropField;
use crate::ui::tooltip::Tooltip;

/// First frame of the gesture (the asset opens on frame 2, the screenshot is taken on frame 240).
const START: u32 = 160;

enum Step {
    Move(Vec2),
    Press(MouseButton),
    Release(MouseButton),
    Wait,
}

#[derive(Default)]
pub struct Script {
    frame: u32,
    steps: Vec<Step>,
    node: Option<String>,
    /// Inline editor the gesture acts on (its value is logged).
    target: Option<PropTarget>,
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
    handles: Query<(&CurveHandle, &UiGlobalTransform, &ComputedNode)>,
    mut events: MessageWriter<WindowEvent>,
    mut moved: MessageWriter<CursorMoved>,
    mut buttons: MessageWriter<MouseButtonInput>,
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
            _ => None,
        };
        let focus_or_popup = palette.open;
        let rev = d.map(|d| d.revision);
        if let Some(c) = editor.curve_doc() {
            info!("demo {name} f{} curve {:?} selection {:?}", s.frame, c.curve.points, c.selection);
            return;
        }
        info!(
            "demo {name} f{} rev {rev:?} shown {:?}: node {:?} model {model:?} canvas {shown:?} selection {selection:?} pan {} palette {focus_or_popup} target {:?} = {value:?}",
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
            "rmb_click" => vec![Step::Move(canvas_min + 20.0), Step::Press(MouseButton::Right), Step::Wait, Step::Wait]
                .into_iter()
                .chain([Step::Release(MouseButton::Right)])
                .collect(),
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
            Some(Step::Wait) => {}
            None if s.frame == START + 40 => report(&s),
            None => {}
        }
    }
}
