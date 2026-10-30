//! Global constant table editor (decision 0017): an add form (name, type, Add) above the table of constants
//! (name, type, value, delete).
//!
//! Widgets are the Properties ones ([`PropField`] + `ConstsProp`), so edits take the usual
//! `SetValue` / `SetComponent` path. The body is rebuilt only when [`ConstsDoc::layout`] changes; value edits keep
//! the widgets, and [`sync_fields`] keeps a constant's number fields and colour picker in step.

use bevy::feathers::controls::{
    ColorInputValue, FeathersColorInput, NumberInputRange, NumberInputValue, SoftLimit, color_input_self_update,
};
use bevy::input::keyboard::KeyboardInput;
use bevy::input_focus::{FocusedInput, InputFocus};
use bevy::prelude::*;
use bevy::ui_widgets::ValueChange;
use shader_graph_core::consts::{ConstType, GlobalConst};

use super::icons::IconId;
use super::props::{PropField, checkbox_w, linear_color, number_input, select_w, text_w, vector_fields};
use super::{TEXT, TEXT_DIM, VIEW_BG, ViewOf, icon_button, icon_tool_button, scroll_column, text};
use crate::consts_doc::ConstsDoc;
use crate::editors::consts::CONSTS;
use editor_core::action::TabAction;
use editor_core::dynval::{PropKey, PropValue};

use crate::ops::prelude::*;
use crate::ops::{Action, Actions, ConstsAction, ConstsProp, Editor, display_value};

/// Width of the name column (logical px).
const NAME_W: f32 = 220.0;
/// Width of the type column.
const TYPE_W: f32 = 140.0;
const ERROR: Color = Color::srgb(0.95, 0.42, 0.42);
const ROW_LINE: Color = Color::srgb_u8(40, 42, 48);

/// The editor topbar row of a consts tab (path, count), rebuilt with the tab.
#[derive(Component, Default, Clone)]
pub(crate) struct ConstsViewBar;

/// Form and table, rebuilt on [`ConstsDoc::layout`].
#[derive(Component, Default, Clone)]
pub struct ConstsViewBody;

pub struct ConstsViewPlugin;

impl Plugin for ConstsViewPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (redraw, sync_fields).chain().after(crate::apply_actions))
            .add_observer(commit_color)
            .add_observer(add_on_enter);
    }
}

pub fn consts_view() -> impl Scene {
    bsn! {
        ViewOf(CONSTS)
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
            // The bottom margin keeps the rows clear of the file type watermark.
            @scroll_column()
            ConstsViewBody
            Node { padding: UiRect::new(px(12), px(12), px(10), px(56)), row_gap: px(4) }
        ]
    }
}

fn cell(width: Option<f32>, content: Box<dyn Scene>) -> Box<dyn Scene> {
    match width {
        Some(w) => Box::new(bsn! {
            Node { width: px(w), flex_shrink: 0.0, align_items: AlignItems::Center } Children [ @{content} ]
        }),
        None => Box::new(bsn! {
            Node { flex_grow: 1.0, min_width: px(0), align_items: AlignItems::Center } Children [ @{content} ]
        }),
    }
}

fn row(cells: Vec<Box<dyn Scene>>, line: bool) -> Box<dyn Scene> {
    let border = if line { 1.0 } else { 0.0 };
    Box::new(bsn! {
        Node {
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            column_gap: px(10),
            min_height: px(34),
            padding: UiRect::vertical(px(3)),
            border: UiRect::bottom(px(border)),
        }
        BorderColor::all(ROW_LINE)
        Children [ {cells} ]
    })
}

fn type_labels() -> Vec<String> {
    ConstType::ALL.iter().map(|t| t.label().to_string()).collect()
}

fn type_select(target: ConstsProp, ty: ConstType) -> Box<dyn Scene> {
    let cur = ConstType::ALL.iter().position(|t| *t == ty).unwrap_or(0);
    select_w(target, type_labels(), Vec::new(), cur)
}

/// Soft limit of an `f32` constant's number input: `0..1` for values in it, otherwise the power of ten around the
/// value (`5` → `0..10`, `-0.5` → `-1..1`, `-250` → `-1000..1000`). Any value can be typed in; the range then widens
/// ([`sync_fields`]).
fn slider_range(v: f32) -> (f32, f32) {
    let m = if v.abs() <= 1.0 || !v.is_finite() { 1.0 } else { 10f32.powf(v.abs().log10().ceil()) };
    (if v < 0.0 { -m } else { 0.0 }, m)
}

/// Editor of a constant's value: `f32` a number input with a soft limit, `vecN` colour picker + component fields,
/// flags a checkbox.
fn value_editor(c: &GlobalConst) -> Box<dyn Scene> {
    let target = PropKey::new(ConstsProp::Value(c.name.clone()));
    match c.ty {
        ConstType::F32 => {
            let value = c.value.as_f32();
            let input = number_input(target, 0, value, Some(slider_range(value)), None, 3);
            Box::new(bsn! {
                Node { flex_direction: FlexDirection::Row, align_items: AlignItems::Center, max_width: px(460), flex_grow: 1.0 }
                Children [ @{input} ]
            })
        }
        ConstType::Vec2 | ConstType::Vec3 | ConstType::Vec4 => {
            let floats = c.value.floats();
            let mut rgba = floats.clone();
            rgba.resize(4, 0.0);
            if c.ty != ConstType::Vec4 {
                rgba[3] = 1.0;
            }
            let (field, color) = (PropField { target: target.clone(), ..default() }, linear_color(&rgba));
            let mut items: Vec<Box<dyn Scene>> = vec![Box::new(bsn! {
                @FeathersColorInput
                ColorInputValue(color)
                on(color_input_self_update)
                field
            })];
            items.extend(vector_fields(&target, &floats, true));
            Box::new(bsn! {
                Node { flex_direction: FlexDirection::Row, column_gap: px(3), align_items: AlignItems::Center, max_width: px(460), flex_grow: 1.0 }
                Children [ {items} ]
            })
        }
        ConstType::Bool | ConstType::CompileFlag => checkbox_w(target, c.value.as_bool()),
    }
}

fn form(c: &ConstsDoc) -> Vec<Box<dyn Scene>> {
    let mut out: Vec<Box<dyn Scene>> = vec![Box::new(text("Add a constant", TEXT_DIM, 12.0))];
    let add = Action::from(ConstsAction::Add);
    out.push(row(
        vec![
            cell(Some(NAME_W), text_w(ConstsProp::DraftName, c.draft_name.clone(), false)),
            cell(Some(TYPE_W), type_select(ConstsProp::DraftType, c.draft_ty)),
            cell(
                None,
                Box::new(icon_tool_button(
                    IconId::Plus,
                    "Add",
                    add,
                    "Add the constant (names are unique project-wide)",
                )),
            ),
        ],
        false,
    ));
    if let Some(e) = &c.form_error {
        out.push(Box::new(text(e.clone(), ERROR, 12.0)));
    }
    out
}

fn table(c: &ConstsDoc) -> Vec<Box<dyn Scene>> {
    let header = |s: &'static str| -> Box<dyn Scene> { Box::new(text(s, TEXT_DIM, 12.0)) };
    let mut out: Vec<Box<dyn Scene>> = vec![Box::new(bsn! { Node { height: px(12) } })];
    out.push(row(
        vec![cell(Some(NAME_W), header("Name")), cell(Some(TYPE_W), header("Type")), cell(None, header("Value"))],
        true,
    ));
    if c.table.consts.is_empty() {
        out.push(Box::new(text("No constants yet: add one above.", TEXT_DIM, 13.0)));
    }
    for k in &c.table.consts {
        let delete = Action::from(ConstsAction::Delete { name: k.name.clone(), confirmed: false });
        out.push(row(
            vec![
                cell(Some(NAME_W), Box::new(text(k.name.clone(), TEXT, 14.0))),
                cell(Some(TYPE_W), type_select(ConstsProp::Type(k.name.clone()), k.ty)),
                cell(None, value_editor(k)),
                Box::new(icon_button(IconId::Delete, delete, "Delete the constant (asks when nodes read it)")),
            ],
            true,
        ));
    }
    out
}

fn redraw(
    editor: Res<Editor>,
    bar: Query<Entity, With<ConstsViewBar>>,
    body: Query<Entity, With<ConstsViewBody>>,
    mut last: Local<Option<u64>>,
    mut commands: Commands,
) {
    let Some(c) = editor.consts_doc() else { return };
    if *last == Some(c.layout) {
        return;
    }
    let (Ok(bar), Ok(body)) = (bar.single(), body.single()) else { return };
    *last = Some(c.layout);
    commands.entity(bar).despawn_children();
    // Only the path: the constant count is in the editor statusbar, how values are used in the sidebar's Usage
    // subpane (design §6.4).
    commands.spawn_scene(bsn! { @text(c.id.source_rel(), TEXT, 13.0) ChildOf(bar) });
    commands.entity(body).despawn_children();
    for r in form(c).into_iter().chain(table(c)) {
        commands.spawn_scene(bsn! { @{r} ChildOf(body) });
    }
}

/// Keeps the number fields and the colour picker of a constant in step with the model (an edit in one of them
/// changes the others); the field being typed into is left alone. A typed value outside an `f32` field's soft limit
/// widens the limit.
fn sync_fields(
    editor: Res<Editor>,
    focus: Option<Res<InputFocus>>,
    numbers: Query<(Entity, &PropField, &NumberInputValue, Option<&SoftLimit>)>,
    pickers: Query<(Entity, &PropField, &ColorInputValue)>,
    parents: Query<&ChildOf>,
    mut commands: Commands,
) {
    if !editor.is_changed() || editor.consts_doc().is_none() {
        return;
    }
    let typing = focus.and_then(|f| f.get()).and_then(|f| parents.get(f).ok()).map(ChildOf::parent);
    for (e, field, shown, soft) in &numbers {
        if !matches!(field.target.downcast(), Some(ConstsProp::Value(_))) {
            continue;
        }
        let Some(v) = editor.current_value(&field.target) else { continue };
        let want = NumberInputValue::F32(display_value(v.floats().get(field.comp).copied().unwrap_or(0.0)));
        if let Some(SoftLimit(NumberInputRange::F32(range))) = soft
            && let NumberInputValue::F32(x) = want
            && !range.contains(&x)
        {
            let (min, max) = slider_range(x);
            // Re-inserting the value redraws the fill bar for the new range; the edit that left the range has
            // finished (typed values arrive on Enter or focus loss), so the field is not being typed into.
            commands.entity(e).try_insert((SoftLimit(NumberInputRange::F32(min..=max)), want));
            continue;
        }
        if Some(e) != typing && *shown != want {
            commands.entity(e).try_insert(want);
        }
    }
    for (e, field, shown) in &pickers {
        if !matches!(field.target.downcast(), Some(ConstsProp::Value(_))) {
            continue;
        }
        let Some(v) = editor.current_value(&field.target) else { continue };
        let mut rgba = v.floats();
        rgba.resize(4, 0.0);
        if !matches!(v, PropValue::Float4(_)) {
            rgba[3] = 1.0;
        }
        let want = linear_color(&rgba);
        let (a, b) = (shown.0.to_linear().to_f32_array(), want.to_linear().to_f32_array());
        if a.iter().zip(b).any(|(x, y)| (x - y).abs() > 1e-5) {
            commands.entity(e).try_insert(ColorInputValue(want));
        }
    }
}

/// A finished picker drag on a constant is one undo step (number fields end theirs with `is_final`).
fn commit_color(ev: On<ValueChange<Color>>, fields: Query<&PropField>, mut actions: ResMut<Actions>) {
    if ev.is_final && fields.get(ev.source).is_ok_and(|f| matches!(f.target.downcast(), Some(ConstsProp::Value(_)))) {
        actions.push(Action::Tab(TabAction::CommitInline));
    }
}

/// Enter in the add form's name field adds the constant.
fn add_on_enter(ev: On<FocusedInput<KeyboardInput>>, fields: Query<&PropField>, mut actions: ResMut<Actions>) {
    if ev.input.key_code == KeyCode::Enter
        && ev.input.state.is_pressed()
        && fields.get(ev.focused_entity).is_ok_and(|f| f.target == PropKey::new(ConstsProp::DraftName))
    {
        actions.push(Action::from(ConstsAction::Add));
    }
}

#[cfg(test)]
mod tests {
    use super::slider_range;

    #[test]
    fn slider_range_covers_the_value() {
        assert_eq!(slider_range(0.5), (0.0, 1.0));
        assert_eq!(slider_range(1.0), (0.0, 1.0));
        assert_eq!(slider_range(5.0), (0.0, 10.0));
        assert_eq!(slider_range(-0.5), (-1.0, 1.0));
        assert_eq!(slider_range(-250.0), (-1000.0, 1000.0));
        assert_eq!(slider_range(f32::NAN), (0.0, 1.0));
    }
}
