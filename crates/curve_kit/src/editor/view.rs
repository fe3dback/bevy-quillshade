//! The curve editor's content and editor topbar (decision 0015): grid and curve rasterized with tiny-skia into an
//! image, points as absolutely positioned UI nodes (so picking, hover and drags come from `bevy_picking`).
//!
//! Click selects (Shift toggles), drag moves the selection (one undo step; X / Y held: along that axis only, Shift
//! held: the grabbed point snaps to the 0.1 grid), double click on empty space adds a point, right click opens a
//! menu. The value axis labels are in the curve's output range (decision 0038); points move in normalized space.
//! Keys (arrow nudging, undo, copy/paste, delete, select-all) are the curve editor's input context
//! ([`super::CurveCtx`]), routed to the curve by `Editor`.
//!
//! Every component, resource, system and observer is `D`'s own: two curve domains each drive only their view.

use std::collections::BTreeSet;
use std::marker::PhantomData;

use bevy::prelude::*;
use bevy::ui::ui_transform::UiGlobalTransform;
use bevy_enhanced_input::prelude::ModKeys;
use editor_core::action::{Action, Actions, TabAction};
use editor_core::dynval::{PropValue, display_value};
use editor_core::editors::ViewOf;
use editor_core::keys::mods;
use editor_core::state::editor::Editor;
use editor_core::theme::{TEXT, TEXT_DIM, text};
use editor_core::ui::dialog::open_icon_menu;
use editor_core::ui::icons::IconId;
use editor_core::ui::props::set_action;
use editor_core::ui::shell::{VIEW_BG, clipped_text};
use editor_core::ui::widgets::{Caption, GroupButton, button_group};
use editor_core::ui::{icon_tool_button, tool_button};
use editor_core::util::geom::{local_pos, logical_size};
use editor_core::util::raster::{opaque_image, skia_color};
use resvg::tiny_skia;

use super::doc::moved;
use super::{CURVE_ICON, CurveAction, CurveDomain, CurveEdit, CurveProp, active, curve_key};
use crate::{Curve, Interp};

/// Space around the unit square for the axis labels (logical px): left, right, top, bottom.
const MARGIN: [f32; 4] = [56.0, 32.0, 28.0, 40.0];
/// Point handle radius (logical px).
const HANDLE: f32 = 6.0;
/// The curve's colour (also the selected points').
pub const CURVE_COLOR: Color = Color::srgb_u8(250, 204, 92);
const POINT_COLOR: Color = Color::srgb(0.86, 0.87, 0.90);
/// Grid cells per unit: a Shift-drag snaps to the plot's grid lines (0.1).
const GRID: f32 = 10.0;

marker! {
    /// The editor topbar row of `D`'s curve tab (path, select all / delete, preview switch, the domain's extras),
    /// rebuilt with the tab or the preview mode.
    CurveViewBar
}

marker! {
    /// The plot area: the unit square plus margins. Receives clicks on empty space.
    CurveCanvas
}

marker! {
    /// Image with grid and curve, filling [`CurveCanvas`].
    CurvePlotImage
}

/// Handle of the point with this index.
#[derive(Component)]
pub struct CurveHandle<D> {
    index: usize,
    hovered: bool,
    domain: PhantomData<fn() -> D>,
}

impl<D> CurveHandle<D> {
    pub fn index(&self) -> usize {
        self.index
    }
}

/// Axis label: `(along t, value)` placed next to the plot.
#[derive(Component)]
struct AxisLabel<D>(bool, f32, PhantomData<fn() -> D>);

/// The drag in progress: points and selection when it started, and the grabbed point.
#[derive(Resource)]
struct CurveDrag<D>(Option<(Vec<(f32, f32)>, BTreeSet<usize>, usize)>, PhantomData<fn() -> D>);

impl<D> Default for CurveDrag<D> {
    fn default() -> Self {
        CurveDrag(None, PhantomData)
    }
}

/// The window systems of `D`'s curve editor: its editor topbar, the plot and the point gestures.
pub struct CurveEditorPlugin<D>(PhantomData<fn() -> D>);

impl<D> Default for CurveEditorPlugin<D> {
    fn default() -> Self {
        CurveEditorPlugin(PhantomData)
    }
}

impl<D: CurveDomain> Plugin for CurveEditorPlugin<D> {
    fn build(&self, app: &mut App) {
        app.init_resource::<CurveDrag<D>>()
            // After the actions of this frame's gestures are applied: a drag shows without a frame of lag.
            .add_systems(
                Update,
                (redraw_bar::<D>, render_plot::<D>, place_handles::<D>).after(editor_core::apply_actions),
            )
            .add_observer(handle_press::<D>)
            .add_observer(handle_drag_start::<D>)
            .add_observer(handle_drag::<D>)
            .add_observer(handle_drag_end::<D>)
            .add_observer(handle_hover::<D, PointerOver, true>)
            .add_observer(handle_hover::<D, PointerOut, false>)
            .add_observer(canvas_click::<D>);
    }
}

/// `D`'s curve view in the editor content.
pub fn curve_view<D: CurveDomain>() -> impl Scene {
    let (view, canvas, image) = (ViewOf(D::VIEW), CurveCanvas::<D>::default(), CurvePlotImage::<D>::default());
    bsn! {
        view
        Node {
            position_type: PositionType::Absolute,
            left: px(0),
            right: px(0),
            top: px(0),
            bottom: px(0),
            flex_direction: FlexDirection::Column,
        }
        BackgroundColor(VIEW_BG)
        Visibility::Hidden
        Children [
            // The bottom margin keeps the plot clear of the file type watermark.
            canvas
            Node { flex_grow: 1.0, min_height: px(0), margin: UiRect::bottom(px(56)) }
            Children [
                image
                ImageNode
                Node { position_type: PositionType::Absolute, left: px(0), right: px(0), top: px(0), bottom: px(0) }
                Pickable::IGNORE
            ]
        ]
    }
}

/// Unit square inside the canvas, in canvas-local logical px.
#[derive(Clone, Copy)]
struct Plot {
    origin: Vec2,
    size: Vec2,
}

impl Plot {
    fn new(canvas: Vec2) -> Plot {
        let [l, r, t, b] = MARGIN;
        Plot { origin: Vec2::new(l, t), size: Vec2::new((canvas.x - l - r).max(1.0), (canvas.y - t - b).max(1.0)) }
    }

    fn to_local(self, (t, v): (f32, f32)) -> Vec2 {
        self.origin + Vec2::new(t * self.size.x, (1.0 - v) * self.size.y)
    }

    fn to_curve(self, local: Vec2) -> (f32, f32) {
        let n = (local - self.origin) / self.size;
        (n.x.clamp(0.0, 1.0), (1.0 - n.y).clamp(0.0, 1.0))
    }

    /// A pointer movement in logical px as a curve-space delta.
    fn delta(self, d: Vec2) -> (f32, f32) {
        (d.x / self.size.x, -d.y / self.size.y)
    }
}

/// `TabAction::SetValue` of the preview mode `interp` on `D`'s curve tab.
fn preview_action<D: CurveDomain>(interp: Interp) -> Action {
    set_action(&curve_key::<D>(CurveProp::Preview), PropValue::Str(interp.label().into()), false)
}

fn redraw_bar<D: CurveDomain>(
    editor: Res<Editor>,
    bar: Query<Entity, With<CurveViewBar<D>>>,
    mut last: Local<Option<(String, Interp)>>,
    mut commands: Commands,
) {
    let Some(c) = active::<D>(&editor) else { return };
    let key = (c.aref.path.clone(), c.preview);
    if last.as_ref() == Some(&key) {
        return;
    }
    let Ok(bar) = bar.single() else { return };
    *last = Some(key);
    commands.entity(bar).despawn_children();
    commands.spawn_scene(bsn! { @clipped_text(c.aref.source_rel(), TEXT, 13.0) ChildOf(bar) });
    let tools: [Box<dyn Scene>; 2] = [
        Box::new(tool_button("Select all", Action::Tab(TabAction::SelectAll))),
        Box::new(icon_tool_button(
            IconId::Delete,
            "Delete",
            Action::Tab(TabAction::DeleteSelection),
            "Delete the selected points (Del)",
        )),
    ];
    for b in tools {
        commands.spawn_scene(bsn! { @{b} Node { flex_shrink: 0.0 } ChildOf(bar) });
    }
    commands.spawn_scene(bsn! { Node { flex_grow: 1.0 } ChildOf(bar) });
    commands.spawn_scene(bsn! { @text("Preview:", TEXT_DIM, 12.0) ChildOf(bar) });
    let modes = Interp::ALL
        .into_iter()
        .map(|interp| GroupButton {
            caption: Caption::Text(interp.label().into()),
            action: preview_action::<D>(interp),
            selected: interp == c.preview,
            disabled: false,
            tip: format!("Preview the {} interpolation", interp.label()),
        })
        .collect();
    commands.spawn_scene(bsn! { @button_group(modes) ChildOf(bar) });
    for extra in D::topbar_extras(c) {
        commands.spawn_scene(bsn! { @{extra} ChildOf(bar) });
    }
}

/// Background and grid of one canvas size, reused while only the curve changes (a drag redraws every frame).
#[derive(Default)]
struct PlotCache {
    size: UVec2,
    base: Option<tiny_skia::Pixmap>,
    image: Option<Handle<Image>>,
}

/// Re-rasterizes the plot when the curve, the preview mode or the canvas size changes.
fn render_plot<D: CurveDomain>(
    editor: Res<Editor>,
    canvas: Query<&ComputedNode, With<CurveCanvas<D>>>,
    mut image_node: Query<&mut ImageNode, With<CurvePlotImage<D>>>,
    mut images: ResMut<Assets<Image>>,
    mut cache: Local<PlotCache>,
    mut last: Local<Option<(String, u64, UVec2, Vec<(f32, f32)>, Interp)>>,
) {
    let Some(c) = active::<D>(&editor) else { return };
    let (Ok(node), Ok(mut image_node)) = (canvas.single(), image_node.single_mut()) else { return };
    let size = node.size().as_uvec2();
    if size.x == 0 || size.y == 0 {
        return;
    }
    // Points and preview too: a reopened document starts at revision 1 again.
    let key = (c.aref.path.clone(), c.revision, size, c.curve.points.clone(), c.preview);
    if last.as_ref() == Some(&key) {
        return;
    }
    *last = Some(key);
    let scale = 1.0 / node.inverse_scale_factor();
    if cache.size != size || cache.base.is_none() {
        cache.size = size;
        cache.base = Some(plot_base(size, scale));
    }
    let mut pixmap = cache.base.clone().expect("base layer");
    draw_curves(&mut pixmap, &c.curve, c.preview, scale);
    // One asset, replaced in place: no new texture handle per frame.
    let handle = cache.image.get_or_insert_with(|| images.reserve_handle()).clone();
    let _ = images.insert(&handle, opaque_image(pixmap));
    if image_node.image != handle {
        image_node.image = handle;
    }
}

/// Opaque background, the unit square and its grid; `scale` = physical px per logical px.
fn plot_base(size: UVec2, scale: f32) -> tiny_skia::Pixmap {
    let mut pixmap = tiny_skia::Pixmap::new(size.x, size.y).expect("non-empty plot size");
    pixmap.fill(skia_color(VIEW_BG, 1.0));
    let plot = Plot::new(size.as_vec2() / scale);
    let px = |p: Vec2| p * scale;
    let mut paint = tiny_skia::Paint { anti_alias: true, ..default() };
    let id = tiny_skia::Transform::identity();
    let (o, s) = (px(plot.origin), px(plot.size));
    if let Some(r) = tiny_skia::Rect::from_xywh(o.x, o.y, s.x, s.y) {
        paint.set_color(skia_color(Color::srgb_u8(30, 32, 37), 1.0));
        pixmap.fill_rect(r, &paint, id, None);
    }
    for i in 0..=10 {
        let f = i as f32 / 10.0;
        let alpha = if i % 5 == 0 { 0.16 } else { 0.06 };
        paint.set_color(skia_color(Color::WHITE, alpha));
        let stroke = tiny_skia::Stroke { width: scale, ..default() };
        for (a, b) in [((f, 0.0), (f, 1.0)), ((0.0, f), (1.0, f))] {
            let (a, b) = (px(plot.to_local(a)), px(plot.to_local(b)));
            let mut pb = tiny_skia::PathBuilder::new();
            pb.move_to(a.x, a.y);
            pb.line_to(b.x, b.y);
            if let Some(path) = pb.finish() {
                pixmap.stroke_path(&path, &paint, &stroke, id, None);
            }
        }
    }
    pixmap
}

/// The curve in `preview` mode and, faintly under it, in the other mode.
fn draw_curves(pixmap: &mut tiny_skia::Pixmap, curve: &Curve, preview: Interp, scale: f32) {
    let plot = Plot::new(Vec2::new(pixmap.width() as f32, pixmap.height() as f32) / scale);
    let mut paint = tiny_skia::Paint { anti_alias: true, ..default() };
    let steps = (plot.size.x / 2.0).max(16.0) as usize;
    let slopes = curve.slopes();
    let other = Interp::ALL.into_iter().find(|i| *i != preview);
    for (interp, alpha, width) in other.map(|i| (i, 0.22, 1.0)).into_iter().chain([(preview, 1.0, 2.0)]) {
        let mut pb = tiny_skia::PathBuilder::new();
        for k in 0..=steps {
            let t = k as f32 / steps as f32;
            let p = plot.to_local((t, curve.sample_normalized_with(t, interp, &slopes))) * scale;
            if k == 0 { pb.move_to(p.x, p.y) } else { pb.line_to(p.x, p.y) }
        }
        if let Some(path) = pb.finish() {
            paint.set_color(skia_color(CURVE_COLOR, alpha));
            let stroke = tiny_skia::Stroke { width: width * scale, line_join: tiny_skia::LineJoin::Round, ..default() };
            pixmap.stroke_path(&path, &paint, &stroke, tiny_skia::Transform::identity(), None);
        }
    }
}

/// An axis label's text: 3 decimals at most, no trailing zeros (`0`, `0.5`, `-2`).
fn axis_text(v: f32) -> String {
    format!("{}", display_value(v) + 0.0)
}

/// The axis labels; the value axis ones get their text (real units) in [`place_handles`].
fn spawn_axis_labels<D: CurveDomain>(canvas: Entity, commands: &mut Commands) {
    for (along_t, v) in [(true, 0.0), (true, 0.5), (true, 1.0), (false, 0.0), (false, 0.5), (false, 1.0)] {
        commands.spawn((
            AxisLabel::<D>(along_t, v, PhantomData),
            Text::new(axis_text(v)),
            TextFont { font_size: FontSize::Px(12.0), ..default() },
            TextColor(TEXT_DIM),
            Node { position_type: PositionType::Absolute, ..default() },
            Pickable::IGNORE,
            ChildOf(canvas),
        ));
    }
}

/// Keeps one handle per point (positioned, colored by selection/hover) and places the axis labels.
fn place_handles<D: CurveDomain>(
    editor: Res<Editor>,
    canvas: Query<(Entity, &ComputedNode), With<CurveCanvas<D>>>,
    mut handles: Query<(Entity, &CurveHandle<D>, &mut Node, &mut BackgroundColor, &mut BorderColor)>,
    mut labels: Query<(&AxisLabel<D>, &mut Node, &ComputedNode, &mut Text), Without<CurveHandle<D>>>,
    mut commands: Commands,
) {
    let Some(c) = active::<D>(&editor) else { return };
    let Ok((canvas, node)) = canvas.single() else { return };
    if labels.is_empty() {
        spawn_axis_labels::<D>(canvas, &mut commands);
    }
    let plot = Plot::new(logical_size(node));
    let n = c.curve.points.len();
    let mut have = vec![false; n];
    for (e, h, mut node, mut bg, mut border) in &mut handles {
        let Some(p) = c.curve.points.get(h.index) else {
            commands.entity(e).despawn();
            continue;
        };
        have[h.index] = true;
        let at = plot.to_local(*p);
        let r = if h.hovered { HANDLE + 1.5 } else { HANDLE };
        (node.left, node.top, node.width, node.height) = (px(at.x - r), px(at.y - r), px(2.0 * r), px(2.0 * r));
        let selected = c.selection.contains(&h.index);
        bg.set_if_neq(BackgroundColor(if selected { CURVE_COLOR } else { POINT_COLOR }));
        let edge = if h.hovered || selected { Color::WHITE } else { Color::srgb_u8(20, 21, 24) };
        border.set_if_neq(BorderColor::all(edge));
    }
    for index in (0..n).filter(|i| !have[*i]) {
        commands.spawn((
            CurveHandle::<D> { index, hovered: false, domain: PhantomData },
            Node {
                position_type: PositionType::Absolute,
                border: UiRect::all(px(1.5)),
                border_radius: BorderRadius::all(percent(50)),
                ..default()
            },
            BackgroundColor(POINT_COLOR),
            BorderColor::all(Color::BLACK),
            ChildOf(canvas),
        ));
    }
    for (label, mut node, computed, mut text) in &mut labels {
        if !label.0 {
            // The value axis is in real units (the curve's output range, decision 0038).
            let real = axis_text(c.curve.to_real(label.1));
            if text.0 != real {
                text.0 = real;
            }
        }
        let size = logical_size(computed);
        let at = if label.0 {
            plot.to_local((label.1, 0.0)) + Vec2::new(-size.x / 2.0, 8.0)
        } else {
            plot.to_local((0.0, label.1)) + Vec2::new(-size.x - 10.0, -size.y / 2.0)
        };
        (node.left, node.top) = (px(at.x), px(at.y));
    }
}

/// LMB on a point: select it (Shift: toggle). RMB: its menu.
fn handle_press<D: CurveDomain>(
    mut ev: On<PointerPress>,
    handles: Query<&CurveHandle<D>>,
    keys: Res<ButtonInput<KeyCode>>,
    editor: Res<Editor>,
    mut actions: ResMut<Actions>,
    mut commands: Commands,
) {
    let Ok(h) = handles.get(ev.event_target()) else { return };
    ev.propagate(false);
    let Some(c) = active::<D>(&editor) else { return };
    let toggle = mods(&keys).contains(ModKeys::SHIFT);
    let mut sel = c.selection.clone();
    match ev.button {
        PointerButton::Primary if toggle => {
            if !sel.remove(&h.index) {
                sel.insert(h.index);
            }
        }
        PointerButton::Primary | PointerButton::Secondary if !sel.contains(&h.index) => sel = [h.index].into(),
        _ => {}
    }
    actions.push(Action::from(CurveEdit::<D>::new(CurveAction::Select(sel.clone()))));
    if ev.button == PointerButton::Secondary {
        let label = if sel.len() > 1 { format!("Delete {} points", sel.len()) } else { "Delete point".into() };
        open_icon_menu(
            &mut commands,
            ev.pointer.position,
            vec![(Some(IconId::Delete.into()), label, Action::Tab(TabAction::DeleteSelection))],
        );
    }
}

fn handle_drag_start<D: CurveDomain>(
    mut ev: On<PointerDragStart>,
    handles: Query<&CurveHandle<D>>,
    editor: Res<Editor>,
    mut drag: ResMut<CurveDrag<D>>,
) {
    let Ok(h) = handles.get(ev.event_target()) else { return };
    ev.propagate(false);
    let Some(c) = active::<D>(&editor) else { return };
    if ev.button != PointerButton::Primary {
        return;
    }
    // The press that started the drag may not be applied yet.
    let sel = if c.selection.contains(&h.index) { c.selection.clone() } else { [h.index].into() };
    drag.0 = Some((c.curve.points.clone(), sel, h.index));
}

/// Keys that constrain a point drag (read on every drag step, so they can be pressed mid-drag).
#[derive(Clone, Copy, Default)]
struct DragKeys {
    /// X held: only `t` changes.
    x_only: bool,
    /// Y held: only the value changes.
    y_only: bool,
    /// Shift held: the grabbed point lands on the grid.
    snap: bool,
}

impl DragKeys {
    fn read(keys: &ButtonInput<KeyCode>) -> DragKeys {
        let (x, y) = (keys.pressed(KeyCode::KeyX), keys.pressed(KeyCode::KeyY));
        let snap = mods(keys).contains(ModKeys::SHIFT);
        DragKeys { x_only: x && !y, y_only: y && !x, snap }
    }
}

/// Points of a drag: `from` (the points when it started) with the selection moved by the pointer's `raw` delta,
/// constrained by `keys`; snapping moves the selection so that the `grabbed` point lands on the grid.
fn dragged(
    from: &[(f32, f32)],
    selected: &BTreeSet<usize>,
    grabbed: usize,
    raw: (f32, f32),
    keys: DragKeys,
) -> (Vec<(f32, f32)>, BTreeSet<usize>) {
    let on_grid = |x: f32| (x * GRID).round() / GRID;
    let (snap_t, snap_v) = (keys.snap && !keys.y_only, keys.snap && !keys.x_only);
    let (mut dt, mut dv) = raw;
    if keys.x_only {
        dv = 0.0;
    }
    if keys.y_only {
        dt = 0.0;
    }
    let anchor = from.get(grabbed).copied().unwrap_or_default();
    if snap_t {
        dt = on_grid(anchor.0 + dt) - anchor.0;
    }
    if snap_v {
        dv = on_grid(anchor.1 + dv) - anchor.1;
    }
    let (mut points, selection) = moved(from, selected, (dt, dv));
    // `anchor + (grid - anchor)` may miss the grid value by an ulp: what landed on the grid is put exactly on it.
    for (_, p) in points.iter_mut().enumerate().filter(|(i, _)| selection.contains(i)) {
        for (x, snapped) in [(&mut p.0, snap_t), (&mut p.1, snap_v)] {
            if snapped && (on_grid(*x) - *x).abs() < 1e-4 {
                *x = on_grid(*x);
            }
        }
    }
    (points, selection)
}

fn handle_drag<D: CurveDomain>(
    mut ev: On<PointerDrag>,
    handles: Query<(), With<CurveHandle<D>>>,
    canvas: Query<&ComputedNode, With<CurveCanvas<D>>>,
    keys: Res<ButtonInput<KeyCode>>,
    drag: Res<CurveDrag<D>>,
    mut actions: ResMut<Actions>,
) {
    if !handles.contains(ev.event_target()) {
        return;
    }
    ev.propagate(false);
    if ev.button != PointerButton::Primary {
        return;
    }
    let (Some((from, sel, grabbed)), Ok(node)) = (&drag.0, canvas.single()) else { return };
    let raw = Plot::new(logical_size(node)).delta(ev.distance);
    let (points, selection) = dragged(from, sel, *grabbed, raw, DragKeys::read(&keys));
    actions.push(Action::from(CurveEdit::<D>::new(CurveAction::SetPoints { points, selection, merge: true })));
}

fn handle_drag_end<D: CurveDomain>(
    mut ev: On<PointerDragEnd>,
    handles: Query<(), With<CurveHandle<D>>>,
    mut drag: ResMut<CurveDrag<D>>,
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

fn handle_hover<D: CurveDomain, E: EntityEvent, const ON: bool>(ev: On<E>, mut handles: Query<&mut CurveHandle<D>>) {
    if let Ok(mut h) = handles.get_mut(ev.event_target()) {
        h.hovered = ON;
    }
}

/// Empty space: click clears the selection, double click adds a point, RMB opens the menu.
fn canvas_click<D: CurveDomain>(
    ev: On<PointerClick>,
    canvas: Query<(&ComputedNode, &UiGlobalTransform), With<CurveCanvas<D>>>,
    keys: Res<ButtonInput<KeyCode>>,
    editor: Res<Editor>,
    mut actions: ResMut<Actions>,
    mut commands: Commands,
) {
    let Ok((node, t)) = canvas.get(ev.event_target()) else { return };
    if ev.original_event_target() != ev.event_target() {
        return;
    }
    let Some(c) = active::<D>(&editor) else { return };
    let Some(local) = local_pos(node, t, ev.pointer.position) else { return };
    let at = Plot::new(logical_size(node)).to_curve(local);
    let shift = mods(&keys).contains(ModKeys::SHIFT);
    match ev.button {
        PointerButton::Primary if ev.count >= 2 => {
            actions.push(Action::from(CurveEdit::<D>::new(CurveAction::Add(at))))
        }
        PointerButton::Primary if !shift => {
            actions.push(Action::from(CurveEdit::<D>::new(CurveAction::Select(BTreeSet::new()))))
        }
        PointerButton::Secondary => {
            let mut items = vec![
                (
                    Some(CURVE_ICON),
                    "Add point here".to_string(),
                    Action::from(CurveEdit::<D>::new(CurveAction::Add(at))),
                ),
                (None, "Select all (Ctrl+A)".into(), Action::Tab(TabAction::SelectAll)),
            ];
            if !c.selection.is_empty() {
                items.push((
                    Some(IconId::Delete.into()),
                    "Delete selected (Del)".into(),
                    Action::Tab(TabAction::DeleteSelection),
                ));
            }
            for interp in Interp::ALL.into_iter().filter(|i| *i != c.preview) {
                items.push((None, format!("Preview: {}", interp.label()), preview_action::<D>(interp)));
            }
            open_icon_menu(&mut commands, ev.pointer.position, items);
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FROM: [(f32, f32); 3] = [(0.0, 0.0), (0.37, 0.52), (1.0, 1.0)];

    fn drag(keys: DragKeys) -> Vec<(f32, f32)> {
        dragged(&FROM, &[1].into(), 1, (0.12, -0.2), keys).0
    }

    #[test]
    fn axis_labels_are_short() {
        assert_eq!(["0", "0.5", "1", "-2", "4", "0.333"], [0.0, 0.5, 1.0, -2.0, 4.0, 1.0 / 3.0].map(axis_text));
        assert_eq!(axis_text(-0.0), "0");
    }

    #[test]
    fn held_keys_constrain_point_drags() {
        let free = drag(DragKeys::default());
        assert!((free[1].0 - 0.49).abs() < 1e-6 && (free[1].1 - 0.32).abs() < 1e-6, "{free:?}");
        assert_eq!(drag(DragKeys { x_only: true, ..default() })[1].1, 0.52, "X: the value stays");
        assert_eq!(drag(DragKeys { y_only: true, ..default() })[1].0, 0.37, "Y: t stays");
        assert_eq!(drag(DragKeys { snap: true, ..default() })[1], (0.5, 0.3), "exactly on the grid");
        assert_eq!(drag(DragKeys { snap: true, x_only: true, ..default() })[1], (0.5, 0.52));
        assert_eq!(drag(DragKeys { snap: true, y_only: true, ..default() })[1], (0.37, 0.3));
    }

    #[test]
    fn snapping_moves_the_whole_selection_by_the_grabbed_point() {
        let from = [(0.21, 0.5), (0.43, 0.5)];
        let keys = DragKeys { snap: true, x_only: true, ..default() };
        let (points, _) = dragged(&from, &[0, 1].into(), 0, (0.05, 0.0), keys);
        assert_eq!(points[0].0, 0.3);
        assert!((points[1].0 - 0.52).abs() < 1e-6, "{points:?}");
    }
}
