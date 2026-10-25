//! Curve asset editor (decision 0015): grid and curve rasterized with tiny-skia into an image, points as
//! absolutely positioned UI nodes (so picking, hover and drags come from `bevy_picking`).
//!
//! Click selects (Shift/Ctrl toggles), drag moves the selection (one undo step), double click on empty space adds
//! a point, right click opens a menu, arrow keys nudge. Undo, copy/paste, delete and select-all are the global
//! shortcuts (`ui::input`), routed to the curve by [`Editor`].

use std::collections::BTreeSet;

use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::ui::ui_transform::UiGlobalTransform;
use resvg::tiny_skia;
use shader_graph_core::Value;
use shader_graph_core::curve::{Curve, Interp};

use super::dialog::open_icon_menu;
use super::icons::IconId;
use super::input::not_typing;
use super::{HEADER_BG, TEXT, TEXT_DIM, VIEW_BG, ViewOf, button, icon_button, text};
use crate::curve_doc::CurveDoc;
use crate::state::{Action, Actions, CurveAction, Editor, PropTarget};
use crate::tab::{Lang, Tab, ViewKind};

/// Space around the unit square for the axis labels (logical px): left, right, top, bottom.
const MARGIN: [f32; 4] = [56.0, 32.0, 28.0, 40.0];
/// Point handle radius (logical px).
const HANDLE: f32 = 6.0;
pub(super) const CURVE_COLOR: Color = Color::srgb_u8(250, 204, 92);
const POINT_COLOR: Color = Color::srgb(0.86, 0.87, 0.90);
/// Arrow-key step in curve units (Shift: ten times).
const NUDGE: f32 = 0.01;

/// Header row (path, preview switch), rebuilt with the tab or the preview mode.
#[derive(Component, Default, Clone)]
pub struct CurveViewBar;

/// The plot area: the unit square plus margins. Receives clicks on empty space.
#[derive(Component, Default, Clone)]
pub struct CurveCanvas;

/// Image with grid and curve, filling [`CurveCanvas`].
#[derive(Component, Default, Clone)]
pub struct CurvePlotImage;

/// Handle of the point with this index.
#[derive(Component)]
pub struct CurveHandle {
    index: usize,
    hovered: bool,
}

impl CurveHandle {
    pub fn index(&self) -> usize {
        self.index
    }
}

/// Axis label: `(along t, value)` placed next to the plot.
#[derive(Component)]
struct AxisLabel(bool, f32);

/// The drag in progress: points and selection when it started.
#[derive(Resource, Default)]
struct CurveDrag(Option<(Vec<(f32, f32)>, BTreeSet<usize>)>);

pub struct CurveViewPlugin;

impl Plugin for CurveViewPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CurveDrag>()
            // After the actions of this frame's gestures are applied: a drag shows without a frame of lag.
            .add_systems(
                Update,
                (redraw_bar, render_plot, place_handles, nudge_keys.run_if(not_typing)).after(crate::apply_actions),
            )
            .add_observer(handle_press)
            .add_observer(handle_drag_start)
            .add_observer(handle_drag)
            .add_observer(handle_drag_end)
            .add_observer(handle_hover::<PointerOver, true>)
            .add_observer(handle_hover::<PointerOut, false>)
            .add_observer(canvas_click);
    }
}

pub fn curve_view() -> impl Scene {
    bsn! {
        ViewOf(ViewKind::Curve)
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
            CurveViewBar
            Node {
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: px(8),
                padding: UiRect::new(px(10), px(4), px(2), px(2)),
            }
            BackgroundColor(HEADER_BG)
            --
            // The bottom margin keeps the plot clear of the file type watermark.
            CurveCanvas
            Node { flex_grow: 1.0, min_height: px(0), margin: UiRect::bottom(px(56)) }
            Children [
                CurvePlotImage
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

fn logical_size(node: &ComputedNode) -> Vec2 {
    node.size() * node.inverse_scale_factor()
}

/// Canvas-local logical position of a window position, if it is inside the canvas.
fn local_pos(node: &ComputedNode, t: &UiGlobalTransform, window_pos: Vec2) -> Option<Vec2> {
    let n = node.normalize_point(*t, window_pos / node.inverse_scale_factor())?;
    (n.x.abs() <= 0.5 && n.y.abs() <= 0.5).then(|| (n + 0.5) * logical_size(node))
}

fn active_curve(editor: &Editor) -> Option<&CurveDoc> {
    editor.curve_doc()
}

fn redraw_bar(
    editor: Res<Editor>,
    bar: Query<Entity, With<CurveViewBar>>,
    mut last: Local<Option<(String, Interp)>>,
    mut commands: Commands,
) {
    let Some(c) = active_curve(&editor) else { return };
    let key = (c.id.path.clone(), c.preview);
    if last.as_ref() == Some(&key) {
        return;
    }
    let Ok(bar) = bar.single() else { return };
    *last = Some(key);
    commands.entity(bar).despawn_children();
    commands.spawn_scene(bsn! { @text(c.id.source_rel(), TEXT, 13.0) ChildOf(bar) });
    commands.spawn_scene(bsn! { Node { flex_grow: 1.0 } ChildOf(bar) });
    commands.spawn_scene(bsn! { @text("Preview:", TEXT_DIM, 12.0) ChildOf(bar) });
    for interp in Interp::ALL {
        let action = Action::SetValue {
            target: PropTarget::CurvePreview,
            value: Value::Str(interp.label().into()),
            merge: false,
        };
        let active = interp == c.preview;
        commands.spawn_scene(bsn! { @button(interp.label(), action, active) ChildOf(bar) });
    }
    let wesl = Action::ShowGenerated(c.id.clone(), Lang::Wesl);
    commands.spawn_scene(bsn! { @icon_button(IconId::Code, wesl, "Show generated WESL") ChildOf(bar) });
}

/// Background and grid of one canvas size, reused while only the curve changes (a drag redraws every frame).
#[derive(Default)]
struct PlotCache {
    size: UVec2,
    base: Option<tiny_skia::Pixmap>,
    image: Option<Handle<Image>>,
}

/// Re-rasterizes the plot when the curve, the preview mode or the canvas size changes.
fn render_plot(
    editor: Res<Editor>,
    canvas: Query<&ComputedNode, With<CurveCanvas>>,
    mut image_node: Query<&mut ImageNode, With<CurvePlotImage>>,
    mut images: ResMut<Assets<Image>>,
    mut cache: Local<PlotCache>,
    mut last: Local<Option<(String, u64, UVec2, Vec<(f32, f32)>, Interp)>>,
) {
    let Some(c) = active_curve(&editor) else { return };
    let (Ok(node), Ok(mut image_node)) = (canvas.single(), image_node.single_mut()) else { return };
    let size = node.size().as_uvec2();
    if size.x == 0 || size.y == 0 {
        return;
    }
    // Points and preview too: a reopened document starts at revision 1 again.
    let key = (c.id.path.clone(), c.revision, size, c.curve.points.clone(), c.preview);
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

/// A pixmap with an opaque background as an image: its premultiplied pixels equal the straight ones Bevy expects.
pub(super) fn opaque_image(pixmap: tiny_skia::Pixmap) -> Image {
    let (width, height) = (pixmap.width(), pixmap.height());
    Image::new(
        Extent3d { width, height, depth_or_array_layers: 1 },
        TextureDimension::D2,
        pixmap.take(),
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    )
}

pub(super) fn skia_color(c: Color, alpha: f32) -> tiny_skia::Color {
    let c = c.to_srgba();
    tiny_skia::Color::from_rgba(c.red, c.green, c.blue, c.alpha * alpha).unwrap_or(tiny_skia::Color::BLACK)
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
            let p = plot.to_local((t, curve.sample_with(t, interp, &slopes))) * scale;
            if k == 0 { pb.move_to(p.x, p.y) } else { pb.line_to(p.x, p.y) }
        }
        if let Some(path) = pb.finish() {
            paint.set_color(skia_color(CURVE_COLOR, alpha));
            let stroke = tiny_skia::Stroke { width: width * scale, line_join: tiny_skia::LineJoin::Round, ..default() };
            pixmap.stroke_path(&path, &paint, &stroke, tiny_skia::Transform::identity(), None);
        }
    }
}

fn spawn_axis_labels(canvas: Entity, commands: &mut Commands) {
    for (along_t, v) in [(true, 0.0), (true, 0.5), (true, 1.0), (false, 0.0), (false, 0.5), (false, 1.0)] {
        let label = if v == 0.5 { "0.5".to_string() } else { format!("{v}") };
        commands.spawn((
            AxisLabel(along_t, v),
            Text::new(label),
            TextFont { font_size: FontSize::Px(12.0), ..default() },
            TextColor(TEXT_DIM),
            Node { position_type: PositionType::Absolute, ..default() },
            Pickable::IGNORE,
            ChildOf(canvas),
        ));
    }
}

/// Keeps one handle per point (positioned, colored by selection/hover) and places the axis labels.
fn place_handles(
    editor: Res<Editor>,
    canvas: Query<(Entity, &ComputedNode), With<CurveCanvas>>,
    mut handles: Query<(Entity, &CurveHandle, &mut Node, &mut BackgroundColor, &mut BorderColor)>,
    mut labels: Query<(&AxisLabel, &mut Node, &ComputedNode), Without<CurveHandle>>,
    mut commands: Commands,
) {
    let Some(c) = active_curve(&editor) else { return };
    let Ok((canvas, node)) = canvas.single() else { return };
    if labels.is_empty() {
        spawn_axis_labels(canvas, &mut commands);
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
            CurveHandle { index, hovered: false },
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
    for (label, mut node, computed) in &mut labels {
        let size = logical_size(computed);
        let at = if label.0 {
            plot.to_local((label.1, 0.0)) + Vec2::new(-size.x / 2.0, 8.0)
        } else {
            plot.to_local((0.0, label.1)) + Vec2::new(-size.x - 10.0, -size.y / 2.0)
        };
        (node.left, node.top) = (px(at.x), px(at.y));
    }
}

/// LMB on a point: select it (Shift/Ctrl: toggle). RMB: its menu.
fn handle_press(
    mut ev: On<PointerPress>,
    handles: Query<&CurveHandle>,
    keys: Res<ButtonInput<KeyCode>>,
    editor: Res<Editor>,
    mut actions: ResMut<Actions>,
    mut commands: Commands,
) {
    let Ok(h) = handles.get(ev.event_target()) else { return };
    ev.propagate(false);
    let Some(c) = active_curve(&editor) else { return };
    let toggle =
        keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight, KeyCode::ControlLeft, KeyCode::ControlRight]);
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
    actions.push(Action::Curve(CurveAction::Select(sel.clone())));
    if ev.button == PointerButton::Secondary {
        let label = if sel.len() > 1 { format!("Delete {} points", sel.len()) } else { "Delete point".into() };
        open_icon_menu(
            &mut commands,
            ev.pointer.position,
            vec![(Some(IconId::Delete), label, Action::DeleteSelection)],
        );
    }
}

fn handle_drag_start(
    mut ev: On<PointerDragStart>,
    handles: Query<&CurveHandle>,
    editor: Res<Editor>,
    mut drag: ResMut<CurveDrag>,
) {
    let Ok(h) = handles.get(ev.event_target()) else { return };
    ev.propagate(false);
    let Some(c) = active_curve(&editor) else { return };
    if ev.button != PointerButton::Primary {
        return;
    }
    // The press that started the drag may not be applied yet.
    let sel = if c.selection.contains(&h.index) { c.selection.clone() } else { [h.index].into() };
    drag.0 = Some((c.curve.points.clone(), sel));
}

fn handle_drag(
    mut ev: On<PointerDrag>,
    handles: Query<(), With<CurveHandle>>,
    canvas: Query<&ComputedNode, With<CurveCanvas>>,
    drag: Res<CurveDrag>,
    mut actions: ResMut<Actions>,
) {
    if !handles.contains(ev.event_target()) {
        return;
    }
    ev.propagate(false);
    if ev.button != PointerButton::Primary {
        return;
    }
    let (Some((from, sel)), Ok(node)) = (&drag.0, canvas.single()) else { return };
    let delta = Plot::new(logical_size(node)).delta(ev.distance);
    let (points, selection) = CurveDoc::moved(from, sel, delta);
    actions.push(Action::Curve(CurveAction::SetPoints { points, selection, merge: true }));
}

fn handle_drag_end(
    mut ev: On<PointerDragEnd>,
    handles: Query<(), With<CurveHandle>>,
    mut drag: ResMut<CurveDrag>,
    mut actions: ResMut<Actions>,
) {
    if !handles.contains(ev.event_target()) {
        return;
    }
    ev.propagate(false);
    if drag.0.take().is_some() {
        actions.push(Action::CommitInline);
    }
}

fn handle_hover<E: EntityEvent, const ON: bool>(ev: On<E>, mut handles: Query<&mut CurveHandle>) {
    if let Ok(mut h) = handles.get_mut(ev.event_target()) {
        h.hovered = ON;
    }
}

/// Empty space: click clears the selection, double click adds a point, RMB opens the menu.
fn canvas_click(
    ev: On<PointerClick>,
    canvas: Query<(&ComputedNode, &UiGlobalTransform), With<CurveCanvas>>,
    keys: Res<ButtonInput<KeyCode>>,
    editor: Res<Editor>,
    mut actions: ResMut<Actions>,
    mut commands: Commands,
) {
    let Ok((node, t)) = canvas.get(ev.event_target()) else { return };
    if ev.original_event_target() != ev.event_target() {
        return;
    }
    let Some(c) = active_curve(&editor) else { return };
    let Some(local) = local_pos(node, t, ev.pointer.position) else { return };
    let at = Plot::new(logical_size(node)).to_curve(local);
    let shift =
        keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight, KeyCode::ControlLeft, KeyCode::ControlRight]);
    match ev.button {
        PointerButton::Primary if ev.count >= 2 => actions.push(Action::Curve(CurveAction::Add(at))),
        PointerButton::Primary if !shift => actions.push(Action::Curve(CurveAction::Select(BTreeSet::new()))),
        PointerButton::Secondary => {
            let mut items = vec![
                (Some(IconId::Curve), "Add point here".to_string(), Action::Curve(CurveAction::Add(at))),
                (None, "Select all (Ctrl+A)".into(), Action::SelectAll),
            ];
            if !c.selection.is_empty() {
                items.push((Some(IconId::Delete), "Delete selected (Del)".into(), Action::DeleteSelection));
            }
            for interp in Interp::ALL.into_iter().filter(|i| *i != c.preview) {
                let value = Value::Str(interp.label().into());
                let action = Action::SetValue { target: PropTarget::CurvePreview, value, merge: false };
                items.push((None, format!("Preview: {}", interp.label()), action));
            }
            open_icon_menu(&mut commands, ev.pointer.position, items);
        }
        _ => {}
    }
}

/// Arrow keys move the selected points (Shift: ten times further).
fn nudge_keys(keys: Res<ButtonInput<KeyCode>>, editor: Res<Editor>, mut actions: ResMut<Actions>) {
    let Some(Tab::Curve(c)) = editor.tab() else { return };
    if c.selection.is_empty() {
        return;
    }
    let step = if keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]) { NUDGE * 10.0 } else { NUDGE };
    let delta = [
        (KeyCode::ArrowLeft, (-step, 0.0)),
        (KeyCode::ArrowRight, (step, 0.0)),
        (KeyCode::ArrowUp, (0.0, step)),
        (KeyCode::ArrowDown, (0.0, -step)),
    ]
    .into_iter()
    .find(|(k, _)| keys.just_pressed(*k));
    if let Some((_, d)) = delta {
        actions.push(Action::Curve(CurveAction::Nudge(d)));
    }
}
