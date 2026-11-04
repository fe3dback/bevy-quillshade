//! Gradients editor content (gradient design §8, layout 2 of the approved mockup): a large bar of the selected slot
//! (a [`GradientStrip`] over a checkerboard), color stop handles above it, midpoint diamonds below each segment, a
//! `t` ruler, and under them a scrollable list of read-only color cards ([`GradientCardsList`], [`GradientStopCard`]).
//!
//! Handles and diamonds are absolutely positioned UI nodes (so picking, hover and drags come from `bevy_picking`, as
//! in `curve_kit`'s curve view): a press on a stop selects it, a drag moves its `t` (one undo step, clamped to
//! `0..1`, re-sorted as it passes other stops) or deletes it when pulled upwards past [`DELETE_STOP_UP_PX`]; a drag on
//! a diamond sets its segment's midpoint; a click on the bar or on the empty strip above it adds a stop there (refused
//! past `MAX_STOPS`, with a log message); a press there that moves [`DRAG_THRESHOLD`] or more before its release adds
//! nothing. Keys are the gradients editor's input context (`editors::gradients`).
//!
//! A click on a stop (press and release without passing [`DRAG_THRESHOLD`]) opens Feathers' color picker for it: one
//! hidden `FeathersColorInput` ([`StopColorInput`]) spans the stop's handle and the bar, so the picker's popover sits
//! below the bar (above the handle when there is no room), and is opened through its lazy menu's `MenuEvent`. Its
//! changes are undoable stop edits ([`stop_color_changed`]); while it is open a [`dialog::menu_barrier`] makes an
//! outside click only close it (decision 0034).

use bevy::feathers::controls::{ColorInputValue, FeathersColorInput, FeathersMenuPopup};
use bevy::input_focus::InputFocus;
use bevy::input_focus::tab_navigation::NavAction;
use bevy::prelude::*;
use bevy::ui::ui_transform::UiGlobalTransform;
use bevy::ui_widgets::{MenuAction, MenuEvent, ValueChange};
use shader_graph_core::gradient::{MID_MAX, MID_MIN, Slot, Stop};
use shader_project::AssetId;

use super::gradient_strip::GradientStrip;
use super::props::linear_color;
use super::{ACCENT, BORDER, PANEL_BG, TEXT, TEXT_DIM, VIEW_BG, ViewOf, dialog, scroll_column, text};
use crate::editors::gradients::GRADIENTS;
use crate::gradients_doc::{GradientSel, GradientsDoc};
use editor_core::action::TabAction;
use editor_core::history::MergeKey;

use crate::ops::prelude::*;
use crate::ops::{Action, Actions, Editor, GradientsAction};
use crate::util::geom::{local_pos, logical_size};

/// Height of the strip above the bar holding the stop handles (logical px).
const HANDLES_H: f32 = 24.0;
/// Stop handle size (logical px).
const HANDLE: Vec2 = Vec2::new(12.0, 16.0);
/// Height of the gradient bar.
const BAR_H: f32 = 120.0;
/// Height of the strip below the bar holding the midpoint diamonds.
const MIDS_H: f32 = 20.0;
/// Midpoint diamond side (before its 45° turn).
const DIAMOND: f32 = 9.0;
/// Height of the `t` ruler under the diamonds.
const RULER_H: f32 = 18.0;
/// `t` of the ruler labels.
const RULER: [f32; 5] = [0.0, 0.25, 0.5, 0.75, 1.0];
/// Fill of an unselected diamond.
const DIAMOND_COLOR: Color = Color::srgb(0.62, 0.64, 0.69);
/// A pointer that moved less than this (logical px) since pressing a stop is a click: the stop stays put and the
/// release opens its color picker. Bevy's picking starts a drag on any movement.
pub const DRAG_THRESHOLD: f32 = 3.0;
/// Pulling a dragged stop handle upwards by at least this many logical pixels deletes the stop.
const DELETE_STOP_UP_PX: f32 = 32.0;
/// Top of a stop handle in [`GradientCanvas`].
const HANDLE_TOP: f32 = HANDLES_H - HANDLE.y - 3.0;

/// The handles, the bar, the diamonds and the ruler: `t` runs over its width. Receives clicks on empty space.
#[derive(Component, Default, Clone)]
pub struct GradientCanvas;

/// The gradient bar: a [`GradientStrip`] of the selected slot.
#[derive(Component, Default, Clone)]
pub struct GradientBarImage;

/// "Empty slot" hint over the bar of an empty slot.
#[derive(Component, Default, Clone)]
struct EmptyBarHint;

/// Scrollable list of read-only color cards below [`GradientCanvas`].
#[derive(Component, Default, Clone)]
pub struct GradientCardsList;

/// Read-only color card for stop `.0` in the active gradient slot.
#[derive(Component, Default, Clone, Copy, Debug, PartialEq, Eq)]
pub struct GradientStopCard(pub usize);

/// Handle of the stop with this index in the selected slot. A press selects it and a drag moves it; a click (press
/// and release without a drag) can be told apart through `PointerClick` on the same entity.
#[derive(Component)]
pub struct StopHandle {
    index: usize,
    hovered: bool,
}

impl StopHandle {
    pub fn index(&self) -> usize {
        self.index
    }
}

/// Diamond of the midpoint of segment `index` (stop `index` → `index + 1`).
#[derive(Component)]
pub struct MidHandle {
    index: usize,
    hovered: bool,
}

/// The stop color picker: a hidden `FeathersColorInput` (only its popup shows) over the handle of stop `stop` and
/// the bar, which its popover is placed against. Opened by [`open_stop_color`].
#[derive(Component, Default, Clone)]
pub struct StopColorInput {
    pub stop: usize,
}

/// The [`dialog::menu_barrier`] of the open stop color picker.
#[derive(Component, Default, Clone)]
struct StopColorBarrier;

/// A ruler label at this `t`.
#[derive(Component, Default, Clone)]
struct RulerLabel(f32);

/// A drag in progress.
#[derive(Clone, Copy)]
enum Drag {
    /// Stop `index` grabbed at `from` (its `t`); `moved` once the pointer passed [`DRAG_THRESHOLD`] and an edit was
    /// sent (the stop is selected from then on, at whatever index the re-sort gave it; the release opens no picker).
    Stop { index: usize, from: f32, moved: bool },
    /// Midpoint of segment `seg` grabbed at `from` (its position along `t`).
    Mid { seg: usize, from: f32 },
}

#[derive(Resource, Default)]
struct GradientDrag(Option<Drag>);

/// The pointer pressed on empty [`GradientCanvas`] space has moved [`DRAG_THRESHOLD`] or more: the `PointerClick`
/// bevy_picking sends on its release ends a drag, not a click ([`canvas_click`]).
#[derive(Resource, Default)]
struct CanvasDragged(bool);

pub struct GradientViewPlugin;

impl Plugin for GradientViewPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<GradientDrag>()
            .init_resource::<CanvasDragged>()
            // After the actions of this frame's gestures are applied: a drag shows without a frame of lag.
            .add_systems(
                Update,
                (sync_bar, place_handles, redraw_color_cards, stop_color_popup).after(crate::apply_actions),
            )
            .add_observer(handle_press)
            .add_observer(stop_click)
            .add_observer(stop_color_changed)
            .add_observer(stop_color_focus_root)
            .add_observer(handle_drag_start)
            .add_observer(handle_drag)
            .add_observer(handle_drag_end)
            .add_observer(hover::<PointerOver, true>)
            .add_observer(hover::<PointerOut, false>)
            .add_observer(canvas_press)
            .add_observer(canvas_drag)
            .add_observer(canvas_click);
    }
}

/// The gradients editor content in `ui::DocArea`.
pub fn gradient_view() -> impl Scene {
    let ruler: Vec<Box<dyn Scene>> = RULER
        .into_iter()
        .map(|t| -> Box<dyn Scene> {
            let tag = RulerLabel(t);
            Box::new(bsn! {
                @text(format!("{t}"), TEXT_DIM, 11.0)
                Node { position_type: PositionType::Absolute, top: px(HANDLES_H + BAR_H + MIDS_H) }
                Pickable::IGNORE
                tag
            })
        })
        .collect();
    bsn! {
        ViewOf(GRADIENTS)
        Node {
            position_type: PositionType::Absolute,
            left: px(0),
            right: px(0),
            top: px(0),
            bottom: px(0),
            flex_direction: FlexDirection::Column,
            // Room for the end handles at `t` = 0 / 1; the bottom keeps clear of the file type watermark.
            padding: UiRect::new(px(32), px(32), px(24), px(56)),
            row_gap: px(10),
        }
        BackgroundColor(VIEW_BG)
        Visibility::Hidden
        Children [
            GradientCanvas
            Node {
                height: px(HANDLES_H + BAR_H + MIDS_H + RULER_H),
                margin: UiRect::top(px(16)),
                flex_shrink: 0.0,
            }
            Children [
                GradientBarImage
                GradientStrip
                ImageNode
                Node {
                    position_type: PositionType::Absolute,
                    left: px(0),
                    right: px(0),
                    top: px(HANDLES_H),
                    height: px(BAR_H),
                    border: UiRect::all(px(1)),
                }
                BorderColor::all(BORDER)
                Pickable::IGNORE
                --
                EmptyBarHint
                Node {
                    position_type: PositionType::Absolute,
                    left: px(0),
                    right: px(0),
                    top: px(HANDLES_H),
                    height: px(BAR_H),
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                }
                Pickable::IGNORE
                Visibility::Hidden
                Children [ @text("Empty slot: click the bar to add a stop", TEXT_DIM, 13.0) ]
                --
                {ruler}
                --
                // Hidden: picking and Tab skip it; the popup it opens is `Visibility::Visible` and shows.
                @FeathersColorInput
                StopColorInput
                Node {
                    position_type: PositionType::Absolute,
                    left: px(0),
                    top: px(HANDLE_TOP),
                    width: px(HANDLE.x),
                    height: px(HANDLES_H + BAR_H - HANDLE_TOP),
                }
                Visibility::Hidden
            ]
            --
            @text("Used Colors", TEXT_DIM, 12.0)
            --
            @scroll_column()
            GradientCardsList
            Node { row_gap: px(8), padding: UiRect::bottom(px(24)) }
        ]
    }
}

fn active(editor: &Editor) -> Option<(&GradientsDoc, &Slot)> {
    let g = editor.gradients_doc()?;
    Some((g, g.current()?))
}

/// Points the bar's strip at the selected slot and shows the empty-slot hint.
fn sync_bar(
    editor: Res<Editor>,
    mut bar: Query<&mut GradientStrip, With<GradientBarImage>>,
    mut hint: Query<&mut Visibility, With<EmptyBarHint>>,
) {
    let Some((g, slot)) = active(&editor) else { return };
    for mut strip in &mut bar {
        if strip.collection != g.id.path || strip.slot != slot.id {
            *strip = GradientStrip { collection: g.id.path.clone(), slot: slot.id };
        }
    }
    let want = if slot.is_empty() { Visibility::Inherited } else { Visibility::Hidden };
    for mut v in &mut hint {
        v.set_if_neq(want);
    }
}

/// Position of segment `seg`'s midpoint along `t`; `None` for a segment of zero length (a hard step has no
/// midpoint to drag).
fn mid_t(slot: &Slot, seg: usize) -> Option<f32> {
    let (a, b) = (slot.stops.get(seg)?, slot.stops.get(seg + 1)?);
    (b.t - a.t > 1e-6).then_some(a.t + a.mid * (b.t - a.t))
}

/// A linear RGBA stop color as an opaque UI color.
fn stop_color(c: [f32; 4]) -> Color {
    Color::linear_rgb(c[0], c[1], c[2])
}

type HandleItem<'a> = (Entity, &'a StopHandle, &'a mut Node, &'a mut BackgroundColor, &'a mut BorderColor);
type MidItem<'a> = (Entity, &'a MidHandle, &'a mut Node, &'a mut BackgroundColor, &'a mut Visibility);

/// Keeps one handle per stop and one diamond per segment (positioned, colored by selection / hover) and places the
/// ruler labels.
#[allow(clippy::type_complexity)]
fn place_handles(
    editor: Res<Editor>,
    canvas: Query<(Entity, &ComputedNode), With<GradientCanvas>>,
    mut stops: Query<HandleItem, (Without<MidHandle>, Without<RulerLabel>)>,
    mut mids: Query<MidItem, (Without<StopHandle>, Without<RulerLabel>)>,
    mut labels: Query<(&RulerLabel, &mut Node, &ComputedNode), (Without<StopHandle>, Without<MidHandle>)>,
    mut picker: Query<(&StopColorInput, &mut Node), (Without<StopHandle>, Without<MidHandle>, Without<RulerLabel>)>,
    mut commands: Commands,
) {
    let Some((g, slot)) = active(&editor) else { return };
    let Ok((canvas, node)) = canvas.single() else { return };
    let w = logical_size(node).x;
    let n = slot.stops.len();
    let mut have = vec![false; n];
    for (e, h, mut node, mut bg, mut border) in &mut stops {
        let Some(stop) = slot.stops.get(h.index) else {
            commands.entity(e).despawn();
            continue;
        };
        have[h.index] = true;
        (node.left, node.top) = (px(stop.t * w - HANDLE.x / 2.0), px(HANDLE_TOP));
        bg.set_if_neq(BackgroundColor(stop_color(stop.color)));
        let edge = match (g.selection == Some(GradientSel::Stop(h.index)), h.hovered) {
            (true, _) => ACCENT,
            (false, true) => Color::WHITE,
            (false, false) => TEXT,
        };
        border.set_if_neq(BorderColor::all(edge));
    }
    for index in (0..n).filter(|i| !have[*i]) {
        commands.spawn((
            StopHandle { index, hovered: false },
            Node {
                position_type: PositionType::Absolute,
                width: px(HANDLE.x),
                height: px(HANDLE.y),
                border: UiRect::all(px(2)),
                border_radius: BorderRadius::all(px(2)),
                ..default()
            },
            Outline::new(px(1), px(0), BORDER),
            BackgroundColor(TEXT),
            BorderColor::all(TEXT),
            ChildOf(canvas),
        ));
    }
    let segments = n.saturating_sub(1);
    let mut have = vec![false; segments];
    for (e, m, mut node, mut bg, mut vis) in &mut mids {
        if m.index >= segments {
            commands.entity(e).despawn();
            continue;
        }
        have[m.index] = true;
        let at = mid_t(slot, m.index);
        vis.set_if_neq(if at.is_some() { Visibility::Inherited } else { Visibility::Hidden });
        (node.left, node.top) = (px(at.unwrap_or(0.0) * w - DIAMOND / 2.0), px(HANDLES_H + BAR_H + 5.0));
        let fill = match (g.selection == Some(GradientSel::Mid(m.index)), m.hovered) {
            (true, _) => ACCENT,
            (false, true) => Color::WHITE,
            (false, false) => DIAMOND_COLOR,
        };
        bg.set_if_neq(BackgroundColor(fill));
    }
    for index in (0..segments).filter(|i| !have[*i]) {
        commands.spawn((
            MidHandle { index, hovered: false },
            Node { position_type: PositionType::Absolute, width: px(DIAMOND), height: px(DIAMOND), ..default() },
            UiTransform::from_rotation(Rot2::degrees(45.0)),
            Outline::new(px(1), px(0), BORDER),
            BackgroundColor(DIAMOND_COLOR),
            ChildOf(canvas),
        ));
    }
    for (input, mut node) in &mut picker {
        if let Some(stop) = slot.stops.get(input.stop) {
            let left = px(stop.t * w - HANDLE.x / 2.0);
            if node.left != left {
                node.left = left;
            }
        }
    }
    for (label, mut node, computed) in &mut labels {
        let half = logical_size(computed).x / 2.0;
        // The end labels stay inside the bar's span.
        let left = (label.0 * w - half).clamp(-half.min(4.0), (w - 2.0 * half + half.min(4.0)).max(0.0));
        node.left = px(left);
    }
}

type CardsKey = (u64, AssetId, usize, u64);

/// Populates [`GradientCardsList`] with one read-only [`GradientStopCard`] per stop in the active slot (or an empty
/// hint when the slot has no stops).
fn redraw_color_cards(
    editor: Res<Editor>,
    list: Query<Entity, With<GradientCardsList>>,
    mut last: Local<Option<CardsKey>>,
    mut commands: Commands,
) {
    let Some((g, slot)) = active(&editor) else {
        *last = None;
        return;
    };
    let key = (editor.project_generation, g.id.clone(), g.slot, g.revision);
    if last.as_ref() == Some(&key) {
        return;
    }
    let Ok(list) = list.single() else { return };
    *last = Some(key);
    commands.entity(list).despawn_children();
    if slot.stops.is_empty() {
        commands.spawn_scene(bsn! {
            @text("Empty gradient slot - click the gradient bar above to add a color stop.", TEXT_DIM, 12.0)
            ChildOf(list)
        });
        return;
    }
    for (i, stop) in slot.stops.iter().enumerate() {
        let card = stop_card(i, stop);
        commands.spawn_scene(bsn! { @{card} ChildOf(list) });
    }
}

fn stop_card(i: usize, stop: &Stop) -> impl Scene {
    let card = GradientStopCard(i);
    let srgb_color = stop_color(stop.color);
    let srgba = srgb_color.to_srgba();
    let to_u8 = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    let (r8, g8, b8) = (to_u8(srgba.red), to_u8(srgba.green), to_u8(srgba.blue));
    let hsla = Hsla::from(srgb_color);
    let h = if hsla.saturation <= 1e-6 || !hsla.hue.is_finite() { 0.0 } else { hsla.hue.clamp(0.0, 360.0) };
    let s = hsla.saturation.clamp(0.0, 1.0) * 100.0;
    let l = hsla.lightness.clamp(0.0, 1.0) * 100.0;
    let info1 = format!("Stop #{i}   |   T: {:.3}", stop.t);
    let info2 = format!("RGB: ({r8}, {g8}, {b8})   |   #{r8:02X}{g8:02X}{b8:02X}");
    let info3 = format!("HSL: ({h:.0} deg, {s:.0}%, {l:.0}%)");
    bsn! {
        card
        Node {
            width: percent(100),
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            column_gap: px(12),
            padding: UiRect::all(px(10)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(4)),
        }
        BackgroundColor(PANEL_BG)
        BorderColor::all(BORDER)
        Children [
            Node {
                width: px(64),
                height: px(64),
                flex_shrink: 0.0,
                border: UiRect::all(px(1)),
                border_radius: BorderRadius::all(px(4)),
            }
            BackgroundColor(srgb_color)
            BorderColor::all(BORDER)
            --
            Node {
                width: px(1),
                height: px(64),
                flex_shrink: 0.0,
            }
            BackgroundColor(BORDER)
            --
            Node {
                flex_direction: FlexDirection::Column,
                row_gap: px(4),
            }
            Children [
                @text(info1, TEXT, 13.0)
                --
                @text(info2, TEXT_DIM, 12.0)
                --
                @text(info3, TEXT_DIM, 12.0)
            ]
        ]
    }
}

/// LMB on a stop or a diamond selects it.
fn handle_press(
    mut ev: On<PointerPress>,
    stops: Query<&StopHandle>,
    mids: Query<&MidHandle>,
    mut actions: ResMut<Actions>,
) {
    let target = ev.event_target();
    let sel = match (stops.get(target), mids.get(target)) {
        (Ok(h), _) => GradientSel::Stop(h.index),
        (_, Ok(m)) => GradientSel::Mid(m.index),
        _ => return,
    };
    ev.propagate(false);
    if ev.button == PointerButton::Primary {
        actions.push(Action::from(GradientsAction::Select(Some(sel))));
    }
}

fn handle_drag_start(
    mut ev: On<PointerDragStart>,
    stops: Query<&StopHandle>,
    mids: Query<&MidHandle>,
    editor: Res<Editor>,
    mut drag: ResMut<GradientDrag>,
) {
    let target = ev.event_target();
    if !stops.contains(target) && !mids.contains(target) {
        return;
    }
    ev.propagate(false);
    let Some((_, slot)) = active(&editor) else { return };
    if ev.button != PointerButton::Primary {
        return;
    }
    drag.0 = match (stops.get(target), mids.get(target)) {
        (Ok(h), _) => slot.stops.get(h.index).map(|s| Drag::Stop { index: h.index, from: s.t, moved: false }),
        (_, Ok(m)) => mid_t(slot, m.index).map(|from| Drag::Mid { seg: m.index, from }),
        _ => None,
    };
}

fn handle_drag(
    mut ev: On<PointerDrag>,
    handles: Query<(), Or<(With<StopHandle>, With<MidHandle>)>>,
    canvas: Query<&ComputedNode, With<GradientCanvas>>,
    editor: Res<Editor>,
    mut drag: ResMut<GradientDrag>,
    mut actions: ResMut<Actions>,
) {
    if !handles.contains(ev.event_target()) {
        return;
    }
    ev.propagate(false);
    if ev.button != PointerButton::Primary {
        return;
    }
    let (Some(d), Ok(node), Some((g, slot))) = (drag.0.as_mut(), canvas.single(), active(&editor)) else { return };
    let w = logical_size(node).x.max(1.0);
    let dt = ev.distance.x / w;
    let key: Option<MergeKey> = Some("drag".into());
    match d {
        Drag::Stop { index, from, moved } => {
            if !*moved && ev.distance.length() < DRAG_THRESHOLD {
                return;
            }
            // After the first edit the stop is the selected one, wherever the re-sort put it.
            let i = match g.selection {
                Some(GradientSel::Stop(j)) if *moved => j,
                _ => *index,
            };
            if ev.distance.y <= -DELETE_STOP_UP_PX {
                let remaining: Vec<_> =
                    slot.stops.iter().enumerate().filter(|(idx, _)| *idx != i).map(|(_, s)| *s).collect();
                actions.push(Action::from(GradientsAction::SetStops(remaining, None, key)));
                actions.push(Action::Tab(TabAction::CommitInline));
                drag.0 = None;
                return;
            }
            *moved = true;
            actions.push(Action::from(GradientsAction::SetStopT(i, (*from + dt).clamp(0.0, 1.0), key)));
        }
        Drag::Mid { seg, from } => {
            let (Some(a), Some(b)) = (slot.stops.get(*seg), slot.stops.get(*seg + 1)) else { return };
            let mid = ((*from + dt - a.t) / (b.t - a.t).max(1e-6)).clamp(MID_MIN, MID_MAX);
            actions.push(Action::from(GradientsAction::SetMid(*seg, mid, key)));
        }
    }
}

fn handle_drag_end(
    mut ev: On<PointerDragEnd>,
    handles: Query<(), Or<(With<StopHandle>, With<MidHandle>)>>,
    mut drag: ResMut<GradientDrag>,
    mut actions: ResMut<Actions>,
) {
    if !handles.contains(ev.event_target()) {
        return;
    }
    ev.propagate(false);
    if drag.0.take().is_some() {
        actions.push(Action::Tab(TabAction::CommitInline));
    }
}

/// A click on a stop handle that did not move it (bevy_picking sends `PointerClick` before `PointerDragEnd`, so the
/// drag is still known here) opens the stop's color picker.
fn stop_click(mut ev: On<PointerClick>, stops: Query<&StopHandle>, drag: Res<GradientDrag>, mut commands: Commands) {
    let Ok(h) = stops.get(ev.event_target()) else { return };
    ev.propagate(false);
    if ev.button == PointerButton::Primary && !matches!(drag.0, Some(Drag::Stop { moved: true, .. })) {
        open_stop_color(&mut commands, h.index);
    }
}

/// Opens the color picker of stop `stop` of the selected slot ([`StopColorInput`]), starting from its color (stored
/// linear); a picker already open stays as it is.
pub fn open_stop_color(commands: &mut Commands, stop: usize) {
    commands.queue(move |world: &mut World| {
        let current = world.resource::<Editor>().gradients_doc().and_then(GradientsDoc::current);
        let Some(color) = current.and_then(|s| s.stops.get(stop)).map(|s| linear_color(&s.color)) else { return };
        let mut inputs = world.query_filtered::<(Entity, Option<&Children>), With<StopColorInput>>();
        let Some((input, open)) = inputs
            .iter(world)
            .map(|(e, children)| {
                (e, children.is_some_and(|c| c.iter().any(|c| world.get::<FeathersMenuPopup>(c).is_some())))
            })
            .next()
        else {
            return;
        };
        if open {
            return;
        }
        world.entity_mut(input).insert((StopColorInput { stop }, ColorInputValue(color)));
        world.trigger(MenuEvent { source: input, action: MenuAction::Open(NavAction::First) });
    });
}

/// A change in the stop color picker: the picker shows it (as Feathers' `color_input_self_update`) and the stop gets
/// it as linear RGBA. Changes are merged (key `"color"`) into one undo step until the picker's `is_final` (a drag
/// released, a hex value entered, a recent color picked) or its closing ([`stop_color_popup`]).
fn stop_color_changed(
    ev: On<ValueChange<Color>>,
    inputs: Query<&StopColorInput>,
    mut actions: ResMut<Actions>,
    mut commands: Commands,
) {
    let Ok(input) = inputs.get(ev.source) else { return };
    commands.entity(ev.source).insert(ColorInputValue(ev.value));
    let color = ev.value.to_linear().to_f32_array();
    actions.push(Action::from(GradientsAction::SetStopColor(input.stop, color, Some("color".into()))));
    if ev.is_final {
        actions.push(Action::Tab(TabAction::CommitInline));
    }
}

/// Esc in the picker hands the focus back to the menu's button (Feathers' `FocusRoot`); ours is hidden, so the focus
/// is dropped instead, as for a context menu (a focused hidden button would take Space / Enter).
fn stop_color_focus_root(ev: On<MenuEvent>, inputs: Query<(), With<StopColorInput>>, mut commands: Commands) {
    let input = ev.event_target();
    if !matches!(ev.action, MenuAction::FocusRoot) || !inputs.contains(input) {
        return;
    }
    // Queued: Feathers' own observer on the same entity sets the focus to the button.
    commands.queue(move |world: &mut World| {
        let mut at = world.get_resource::<InputFocus>().and_then(InputFocus::get);
        let mut inside = false;
        while let Some(e) = at {
            inside |= e == input;
            at = world.get::<ChildOf>(e).map(ChildOf::parent);
        }
        if inside && let Some(mut focus) = world.get_resource_mut::<InputFocus>() {
            focus.clear();
        }
    });
}

/// Keeps the stop color picker's [`dialog::menu_barrier`] while its popup is open, closes the popup when its stop is
/// no longer the selected one of an active gradients tab, and ends the picker's merged edits when it closes.
fn stop_color_popup(
    editor: Res<Editor>,
    inputs: Query<(&StopColorInput, &Children)>,
    popups: Query<Entity, With<FeathersMenuPopup>>,
    barriers: Query<Entity, With<StopColorBarrier>>,
    mut was_open: Local<bool>,
    mut actions: ResMut<Actions>,
    mut commands: Commands,
) {
    let mut open = false;
    for (input, children) in &inputs {
        let selected = editor.gradients_doc().is_some_and(|g| g.selection == Some(GradientSel::Stop(input.stop)));
        for popup in children.iter().filter(|c| popups.contains(*c)) {
            if selected {
                open = true;
            } else {
                commands.entity(popup).despawn();
            }
        }
    }
    if open && barriers.is_empty() {
        commands.spawn_scene(bsn! { @dialog::menu_barrier() StopColorBarrier });
    } else if !open {
        for b in &barriers {
            commands.entity(b).despawn();
        }
    }
    if *was_open && !open {
        actions.push(Action::Tab(TabAction::CommitInline));
    }
    *was_open = open;
}

fn hover<E: EntityEvent, const ON: bool>(
    ev: On<E>,
    mut stops: Query<&mut StopHandle>,
    mut mids: Query<&mut MidHandle>,
) {
    if let Ok(mut h) = stops.get_mut(ev.event_target()) {
        h.hovered = ON;
    }
    if let Ok(mut m) = mids.get_mut(ev.event_target()) {
        m.hovered = ON;
    }
}

/// A press on the canvas starts a possible click ([`CanvasDragged`]).
fn canvas_press(ev: On<PointerPress>, canvas: Query<(), With<GradientCanvas>>, mut dragged: ResMut<CanvasDragged>) {
    if canvas.contains(ev.event_target()) {
        dragged.0 = false;
    }
}

/// A drag on the canvas past [`DRAG_THRESHOLD`] turns its release into no click ([`CanvasDragged`]).
fn canvas_drag(ev: On<PointerDrag>, canvas: Query<(), With<GradientCanvas>>, mut dragged: ResMut<CanvasDragged>) {
    if canvas.contains(ev.event_target()) && ev.distance.length() >= DRAG_THRESHOLD {
        dragged.0 = true;
    }
}

/// LMB on the bar or the empty strip above it adds a stop there; below the bar it clears the selection. A release
/// that ends a drag ([`CanvasDragged`]) does neither.
fn canvas_click(
    ev: On<PointerClick>,
    canvas: Query<(&ComputedNode, &UiGlobalTransform), With<GradientCanvas>>,
    editor: Res<Editor>,
    mut dragged: ResMut<CanvasDragged>,
    mut actions: ResMut<Actions>,
) {
    let Ok((node, t)) = canvas.get(ev.event_target()) else { return };
    if std::mem::take(&mut dragged.0) {
        return;
    }
    if ev.original_event_target() != ev.event_target() || ev.button != PointerButton::Primary {
        return;
    }
    if active(&editor).is_none() {
        return;
    }
    let Some(local) = local_pos(node, t, ev.pointer.position) else { return };
    let w = logical_size(node).x.max(1.0);
    let action = if local.y < HANDLES_H + BAR_H {
        GradientsAction::AddStop((local.x / w).clamp(0.0, 1.0))
    } else {
        GradientsAction::Select(None)
    };
    actions.push(Action::from(action));
}

#[cfg(test)]
mod tests {
    use super::*;
    use shader_graph_core::gradient::Stop;

    #[test]
    fn midpoint_sits_inside_its_segment_and_steps_have_none() {
        let stop = |t, mid| Stop { t, color: [1.0; 4], mid };
        let slot = Slot { id: 1, name: String::new(), stops: vec![stop(0.2, 0.25), stop(0.6, 0.5), stop(0.6, 0.5)] };
        assert!((mid_t(&slot, 0).unwrap() - 0.3).abs() < 1e-6);
        assert_eq!(mid_t(&slot, 1), None, "a hard step");
        assert_eq!(mid_t(&slot, 2), None, "no segment after the last stop");
    }
}
